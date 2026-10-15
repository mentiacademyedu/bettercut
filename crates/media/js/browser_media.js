// Files the person gave the page, opened and decoded by the browser itself.
//
// The web build's media layer (`src/web_media.rs`) reads through these few
// functions. A browser decodes only asynchronously, and the editor asks for
// frames synchronously, so this keeps the last frames it decoded and answers
// from them; a frame not ready yet is asked for, and when it arrives
// `changes()` moves on so the app draws again.
//
// Two ways to a frame. Asked for one here and one there — scrubbing,
// stepping, a click on the ruler — the `<video>` seeks to each. Asked for
// frame after frame, a little later each time — playback — it plays, and
// every frame it shows is caught as it is shown (requestVideoFrameCallback),
// which is far cheaper than a seek per frame.
//
// Every file is kept as the browser's own `File` (no copy into WebAssembly
// memory) and read through an object URL: a `<video>` element for video and
// sound, `createImageBitmap` for photos. Frames come out as RGBA, sRGB, at
// most PREVIEW_WIDTH wide — a preview-sized copy, like a desktop proxy.

const PREVIEW_WIDTH = 1280;
// Decoded frames kept per file, newest last.
const FRAMES_KEPT = 24;
// The media browser's thumbnails, and the timeline's filmstrip: TILES
// tiles of TILE_WIDTH, side by side in one sheet — the desktop's layout
// (`bettercut_playback::filmstrip_job`).
const POSTER_WIDTH = 160;
const TILES = 32;
const TILE_WIDTH = 64;
// Sound is decoded whole, at the editor's 48 kHz, when a file is opened;
// files larger than this are opened without their sound rather than held in
// memory twice.
const SOUND_RATE = 48000;
// The timeline's waveform: the desktop's layout (`bettercut_cache::Waveform`),
// this many buckets a second, each the lowest and highest of the sound
// folded to mono, as signed bytes.
const PEAKS_PER_SECOND = 200;
const MAX_SOUND_FILE = 1536 * 1024 * 1024;
// How long one frame covers when the file does not say (not an MP4 or
// MOV), for matching a request to a frame caught while playing.
const FRAME_MS = 1000 / 30;
// The most of an MP4's header read to find its frame rate.
const MAX_MOOV = 64 * 1024 * 1024;
// Requests this close after the last, and later in the file, are playback.
const PLAYBACK_STEP_MS = 250;
// Quiet this long while playing, and the video is paused again.
const IDLE_MS = 400;

// What is kept between visits: every file the open project uses, and the
// project itself, in IndexedDB — which holds a `File` as it is.
const DATABASE = 'bettercut';
const FILES = 'files'; // path -> File
const PROJECTS = 'projects'; // project id -> { json, name, modified }; 'current' -> an id
const KEPT_FILES = 'kept'; // path -> bytes: the editor's own small files (LUTs)

const files = new Map(); // path -> entry
const order = []; // entries in the order they were given, until handed over
const ready = []; // paths ready to import, in that order
let nextId = 1;
let changes = 0;
let pending = 0;

function kindOf(file) {
  const type = file.type || '';
  const name = file.name.toLowerCase();
  if (type.startsWith('image/') || /\.(png|jpe?g|webp|gif|bmp|avif)$/.test(name)) return 'image';
  if (type.startsWith('audio/') || /\.(mp3|wav|m4a|aac|flac|ogg|opus)$/.test(name)) return 'audio';
  return 'video';
}

// Details known: hand over every file given so far whose details are known,
// in the order given — a quick photo does not jump ahead of the video
// dropped before it.
function finish(entry, details) {
  entry.details = details;
  pending -= 1;
  while (order.length && order[0].details) ready.push(order.shift().path);
  changes += 1;
}

// ---- the frame rate, from an MP4 or MOV's own tables -------------------------
//
// A `<video>` does not say its rate, but the file does: an MP4's video
// track's sample table holds how many frames there are and how long they
// last; a WebM's track may say how long each frame is.
// { num, den } frames a second, or null for anything else.

async function bytesAt(file, from, length) {
  return new DataView(await file.slice(from, from + length).arrayBuffer());
}

// The boxes in `view` from `start` to `end`: [{ type, at, size }] — `at` is
// where the box's contents begin.
function boxesIn(view, start, end) {
  const boxes = [];
  let at = start;
  while (at + 8 <= end) {
    let size = view.getUint32(at);
    const type = String.fromCharCode(view.getUint8(at + 4), view.getUint8(at + 5), view.getUint8(at + 6), view.getUint8(at + 7));
    let header = 8;
    if (size === 1) {
      if (at + 16 > end) break;
      size = Number(view.getBigUint64(at + 8));
      header = 16;
    } else if (size === 0) {
      size = end - at;
    }
    if (size < header || at + size > end) break;
    boxes.push({ type, at: at + header, end: at + size });
    at += size;
  }
  return boxes;
}

function child(view, box, type) {
  return box && boxesIn(view, box.at, box.end).find((b) => b.type === type);
}

// A Matroska or WebM file keeps a video track's frame length, in
// nanoseconds, as the track's DefaultDuration — when the muxer wrote it
// (a browser's own recordings often leave it out: their rate varies).
const EBML_HEAD = 4 * 1024 * 1024;

// An EBML number at `at`: { value, length }, with the length marker kept
// for an element's id and taken off for its size. A size of all ones is
// "unknown" (a live recording's Segment), given as Infinity.
function ebmlNumber(view, at, keepMarker) {
  if (at >= view.byteLength) return null;
  const first = view.getUint8(at);
  let length = 1;
  while (length <= 8 && !(first & (0x80 >> (length - 1)))) length++;
  if (length > 8 || at + length > view.byteLength) return null;
  let value = keepMarker ? first : first & (0xff >> length);
  let ones = value === (0xff >> length);
  for (let i = 1; i < length; i++) {
    const byte = view.getUint8(at + i);
    value = value * 256 + byte;
    ones = ones && byte === 0xff;
  }
  if (!keepMarker && ones) value = Infinity;
  return { value, length };
}

// The elements in `view` from `start` to `end`: [{ id, at, end }].
function ebmlIn(view, start, end) {
  const elements = [];
  let at = start;
  while (at < end) {
    const id = ebmlNumber(view, at, true);
    const size = id && ebmlNumber(view, at + id.length, false);
    if (!size) break;
    const body = at + id.length + size.length;
    const stop = size.value === Infinity ? end : body + size.value;
    elements.push({ id: id.value, at: body, end: Math.min(stop, end) });
    if (stop > end) break;
    at = stop;
  }
  return elements;
}

function ebmlUint(view, element) {
  let value = 0;
  for (let i = element.at; i < element.end; i++) value = value * 256 + view.getUint8(i);
  return value;
}

async function webmFrameRateOf(file) {
  const view = await bytesAt(file, 0, Math.min(file.size, EBML_HEAD));
  const top = ebmlIn(view, 0, view.byteLength);
  if (!top.length || top[0].id !== 0x1a45dfa3) return null;
  const segment = top.find((e) => e.id === 0x18538067);
  if (!segment) return null;
  for (const tracks of ebmlIn(view, segment.at, segment.end).filter((e) => e.id === 0x1654ae6b)) {
    for (const track of ebmlIn(view, tracks.at, tracks.end).filter((e) => e.id === 0xae)) {
      const fields = ebmlIn(view, track.at, track.end);
      const type = fields.find((e) => e.id === 0x83);
      const duration = fields.find((e) => e.id === 0x23e383);
      if (type && ebmlUint(view, type) === 1 && duration) {
        const ns = ebmlUint(view, duration);
        if (ns > 0) return { num: 1e9, den: ns };
      }
    }
  }
  return null;
}

async function frameRateOf(file) {
  const magic = await bytesAt(file, 0, 4);
  if (magic.byteLength === 4 && magic.getUint32(0) === 0x1a45dfa3) return webmFrameRateOf(file);
  const moov = await moovOf(file);
  if (!moov) return null;
  for (const trak of boxesIn(moov, 8, moov.byteLength).filter((b) => b.type === 'trak')) {
    const mdia = child(moov, trak, 'mdia');
    const hdlr = child(moov, mdia, 'hdlr');
    if (!hdlr || hdlr.at + 12 > hdlr.end) continue;
    const handler = String.fromCharCode(...[8, 9, 10, 11].map((i) => moov.getUint8(hdlr.at + i)));
    if (handler !== 'vide') continue;
    const mdhd = child(moov, mdia, 'mdhd');
    if (!mdhd) continue;
    const timescale = moov.getUint32(mdhd.at + (moov.getUint8(mdhd.at) === 1 ? 20 : 12));
    const stts = child(moov, child(moov, child(moov, mdia, 'minf'), 'stbl'), 'stts');
    if (!stts || !timescale) continue;
    const entries = moov.getUint32(stts.at + 4);
    let frames = 0;
    let ticks = 0;
    for (let i = 0; i < entries && stts.at + 16 + i * 8 <= stts.end; i++) {
      const count = moov.getUint32(stts.at + 8 + i * 8);
      frames += count;
      ticks += count * moov.getUint32(stts.at + 12 + i * 8);
    }
    // A single still picture in a movie says nothing of a rate.
    if (frames > 1 && ticks > 0) return { num: frames * timescale, den: ticks };
  }
  return null;
}

// An MP4 or MOV's movie header (the `moov` box, header included), or null.
async function moovOf(file) {
  // Find it among the file's top-level boxes.
  let at = 0;
  let moov = null;
  while (at + 8 <= file.size) {
    const head = await bytesAt(file, at, 16);
    if (head.byteLength < 8) break;
    let size = head.getUint32(0);
    const type = String.fromCharCode(head.getUint8(4), head.getUint8(5), head.getUint8(6), head.getUint8(7));
    if (at === 0 && type !== 'ftyp' && type !== 'moov' && type !== 'wide' && type !== 'mdat' && type !== 'free') return null;
    if (size === 1 && head.byteLength >= 16) size = Number(head.getBigUint64(8));
    else if (size === 0) size = file.size - at;
    if (size < 8) return null;
    if (type === 'moov') {
      if (size > MAX_MOOV) return null;
      moov = await bytesAt(file, at, size);
      break;
    }
    at += size;
  }
  return moov;
}

function scaled(width, height) {
  if (width <= PREVIEW_WIDTH) return [width, height];
  return [PREVIEW_WIDTH, Math.max(1, Math.round((height * PREVIEW_WIDTH) / width))];
}

function grab(source, width, height) {
  const [w, h] = scaled(width, height);
  const canvas = new OffscreenCanvas(w, h);
  const context = canvas.getContext('2d', { willReadFrequently: true });
  context.drawImage(source, 0, 0, w, h);
  return { width: w, height: h, data: context.getImageData(0, 0, w, h).data };
}

// ---- frames copied off the page's thread ------------------------------------
//
// Copying a decoded frame into memory is the slow step of showing it — 10 to
// 20 ms for HD — and on the page's own thread it holds up everything drawn
// meanwhile, every frame of playback. So the frame is handed over as a
// VideoFrame to a Worker, which scales and copies it; this thread only hands
// it over. Where that cannot be done, it is copied here as before.

const COPIER = `onmessage = (e) => {
  const { id, frame, width, height } = e.data;
  try {
    const canvas = new OffscreenCanvas(width, height);
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.drawImage(frame, 0, 0, width, height);
    const data = context.getImageData(0, 0, width, height).data;
    postMessage({ id, width, height, data }, [data.buffer]);
  } catch (err) {
    postMessage({ id, error: String(err) });
  } finally {
    frame.close();
  }
};`;
let copier = null;
let copierBroken = false;
let copyNext = 0;
const copying = new Map(); // id -> resolve

function copierOf() {
  if (!copier) {
    copier = new Worker(URL.createObjectURL(new Blob([COPIER], { type: 'text/javascript' })));
    copier.onmessage = (e) => {
      const { id, error, ...picture } = e.data;
      const resolve = copying.get(id);
      copying.delete(id);
      if (error) copierBroken = true;
      resolve?.(error ? null : picture);
    };
  }
  return copier;
}

// The frame `video` shows now, at preview size: a Promise of { width,
// height, data }, or of null if it could not be copied.
function grabLater(video) {
  const width = video.videoWidth;
  const height = video.videoHeight;
  if (!copierBroken && typeof VideoFrame !== 'undefined') {
    let frame = null;
    try {
      frame = new VideoFrame(video, { timestamp: 0 });
    } catch (_) {
      // Not this frame: copied here instead.
    }
    // A frame the file turns is left to the `<video>`, which turns it.
    if (frame && frame.displayWidth === width && frame.displayHeight === height) {
      const [w, h] = scaled(width, height);
      const id = copyNext++;
      return new Promise((resolve) => {
        copying.set(id, resolve);
        copierOf().postMessage({ id, frame, width: w, height: h }, [frame]);
      });
    }
    frame?.close();
  }
  return Promise.resolve(grab(video, width, height));
}

function keep(entry, ms, frame) {
  const frames = entry.frames;
  const at = frames.findIndex((f) => f.ms === ms);
  if (at >= 0) frames.splice(at, 1);
  frames.push({ ms, frame });
  if (frames.length > FRAMES_KEPT) frames.shift();
  entry.last = { ms, frame };
  changes += 1;
}

// Poster and filmstrip, from a second `<video>` so the one the preview
// reads from is never moved: a tile at the centre of each of TILES equal
// stretches, as the desktop samples them, and the poster a tenth of the way
// in, past any fade from black.
function makePictures(entry) {
  const video = document.createElement('video');
  video.muted = true;
  video.preload = 'auto';
  const duration = entry.details.duration;
  const height = Math.max(1, Math.round((TILE_WIDTH * entry.details.height) / entry.details.width));
  const sheet = new OffscreenCanvas(TILES * TILE_WIDTH, height);
  const context = sheet.getContext('2d', { willReadFrequently: true });
  let tile = -1; // -1: the poster
  const next = () => {
    tile += 1;
    if (tile >= TILES) {
      entry.strip = {
        width: sheet.width,
        height: sheet.height,
        data: context.getImageData(0, 0, sheet.width, sheet.height).data,
      };
      video.removeAttribute('src');
      video.load();
      changes += 1;
      return;
    }
    video.currentTime = (duration * (2 * tile + 1)) / (2 * TILES);
  };
  video.addEventListener('seeked', () => {
    if (tile < 0) {
      entry.poster = grabWidth(video, video.videoWidth, video.videoHeight, POSTER_WIDTH);
      changes += 1;
    } else {
      context.drawImage(video, tile * TILE_WIDTH, 0, TILE_WIDTH, height);
    }
    next();
  });
  video.addEventListener('loadedmetadata', () => {
    video.currentTime = duration / 10;
  }, { once: true });
  video.src = entry.url;
}

function grabWidth(source, width, height, target) {
  const w = Math.min(target, width);
  const h = Math.max(1, Math.round((height * w) / width));
  const canvas = new OffscreenCanvas(w, h);
  const context = canvas.getContext('2d', { willReadFrequently: true });
  context.drawImage(source, 0, 0, w, h);
  return { width: w, height: h, data: context.getImageData(0, 0, w, h).data };
}

function openImage(entry) {
  createImageBitmap(entry.file)
    .then((bitmap) => {
      entry.still = grab(bitmap, bitmap.width, bitmap.height);
      entry.poster = grabWidth(bitmap, bitmap.width, bitmap.height, POSTER_WIDTH);
      finish(entry, {
        kind: 'image',
        width: bitmap.width,
        height: bitmap.height,
        duration: 0,
      });
      bitmap.close();
    })
    .catch((err) => finish(entry, { error: String(err) }));
}

// ---- sound, decoded from an MP4 or MOV's own tables -------------------------
//
// The browser's decodeAudioData wants the whole file in memory at once, and
// a long video is more than a page can hold. So an MP4's sound track is read
// a piece at a time, decoded with AudioDecoder, brought to 48 kHz a block at
// a time and kept as 16-bit samples — half the room of the browser's floats.
// Anything else (and an MP4 whose sound this cannot decode) is given to
// decodeAudioData whole, as before.

// Brought to 48 kHz this many seconds at a time.
const SOUND_BLOCK_SECONDS = 10;
// Source samples either side of a block as it is brought to 48 kHz, so the
// blocks join without a seam.
const RESAMPLE_PAD = 512;
// The sound track is read from the file in runs of about this many bytes.
const SOUND_READ_BYTES = 1 << 20;
// Longer than this, a file's sound is not kept: 16-bit stereo at 48 kHz is
// about 700 MB an hour.
const MAX_SOUND_SECONDS = 2 * 3600;

// A descriptor in an esds box: { tag, at, end }.
function descriptorAt(view, at) {
  const tag = view.getUint8(at);
  let length = 0;
  let p = at + 1;
  for (let i = 0; i < 4; i++) {
    const b = view.getUint8(p++);
    length = (length << 7) | (b & 0x7f);
    if (!(b & 0x80)) break;
  }
  return { tag, at: p, end: p + length };
}

// What AudioDecoder needs to be told of a sound sample entry, or null.
function soundConfigOf(view, entry) {
  const version = view.getUint16(entry.at + 8);
  if (version > 1) return null;
  const channels = view.getUint16(entry.at + 16);
  const sampleRate = view.getUint32(entry.at + 24) >>> 16;
  const boxes = boxesIn(view, entry.at + 28 + (version === 1 ? 16 : 0), entry.end);
  const find = (type) =>
    boxes.find((b) => b.type === type) ??
    boxesIn(view, boxes.find((b) => b.type === 'wave')?.at ?? 0, boxes.find((b) => b.type === 'wave')?.end ?? 0).find(
      (b) => b.type === type,
    );
  if (entry.type === 'mp4a') {
    const esds = find('esds');
    if (!esds) return null;
    const es = descriptorAt(view, esds.at + 4);
    if (es.tag !== 3) return null;
    let p = es.at + 2;
    const flags = view.getUint8(p++);
    if (flags & 0x80) p += 2;
    if (flags & 0x40) p += 1 + view.getUint8(p);
    if (flags & 0x20) p += 2;
    const dc = descriptorAt(view, p);
    if (dc.tag !== 4) return null;
    const type = view.getUint8(dc.at);
    if (type === 0x6b || type === 0x69) return { codec: 'mp3', sampleRate, numberOfChannels: channels };
    const dsi = descriptorAt(view, dc.at + 13);
    if (dsi.tag !== 5 || dsi.end <= dsi.at) return null;
    const asc = bytesOf(view, dsi);
    let object = asc[0] >> 3;
    if (object === 31 && asc.length > 1) object = 32 + (((asc[0] & 7) << 3) | (asc[1] >> 5));
    if (type !== 0x40 && type !== 0x66 && type !== 0x67) return null;
    return { codec: `mp4a.40.${type === 0x40 ? object : 2}`, sampleRate, numberOfChannels: channels, description: asc };
  }
  if (entry.type === 'Opus') {
    const dops = find('dOps');
    if (!dops) return null;
    // WebCodecs takes Opus's setup as an OpusHead, little-endian, which
    // the MP4's dOps holds big-endian.
    const head = new DataView(new ArrayBuffer(19));
    'OpusHead'.split('').forEach((c, i) => head.setUint8(i, c.charCodeAt(0)));
    head.setUint8(8, 1);
    head.setUint8(9, view.getUint8(dops.at + 1));
    head.setUint16(10, view.getUint16(dops.at + 2), true);
    head.setUint32(12, view.getUint32(dops.at + 4), true);
    head.setInt16(16, view.getInt16(dops.at + 8), true);
    head.setUint8(18, 0);
    return { codec: 'opus', sampleRate: 48000, numberOfChannels: view.getUint8(dops.at + 1), description: new Uint8Array(head.buffer) };
  }
  return null;
}

// `planes` (Float32Array each) at `rate`, brought to 48 kHz.
async function to48k(planes, rate) {
  if (rate === SOUND_RATE) return planes;
  const length = planes[0].length;
  const out = Math.max(1, Math.round((length * SOUND_RATE) / rate));
  const context = new OfflineAudioContext(planes.length, out, SOUND_RATE);
  const buffer = context.createBuffer(planes.length, length, rate);
  planes.forEach((plane, c) => buffer.copyToChannel(plane, c));
  const source = context.createBufferSource();
  source.buffer = buffer;
  source.connect(context.destination);
  source.start(0);
  const rendered = await context.startRendering();
  return planes.map((_, c) => rendered.getChannelData(c));
}

function toInt16(plane) {
  const out = new Int16Array(plane.length);
  for (let i = 0; i < plane.length; i++) out[i] = Math.max(-1, Math.min(1, plane[i])) * 32767;
  return out;
}

// The sound track of an MP4 or MOV, decoded into `entry.sound`. false when
// this cannot decode it — decodeAudioData is to try instead.
async function decodeSoundTrack(entry) {
  if (typeof AudioDecoder === 'undefined') return false;
  const moov = await moovOf(entry.file);
  if (!moov) return false;
  const { tracks, movieScale } = tracksOf(moov, 'soun');
  if (!tracks.length) {
    entry.sound = 'none';
    return true;
  }
  const samples = sampleTableOf(moov, tracks[0], movieScale);
  const config = samples && soundConfigOf(moov, samples.entry);
  if (!config || !samples.count) return false;
  try {
    if (!(await AudioDecoder.isConfigSupported(config)).supported) return false;
  } catch (_) {
    return false;
  }
  let ticks = 0;
  for (let i = 0; i < samples.count; i++) ticks += samples.dur[i];
  if (ticks / samples.timescale > MAX_SOUND_SECONDS) {
    console.warn(`bettercut: ${entry.file.name} is too long to keep its sound in this browser`);
    entry.sound = 'none';
    return true;
  }

  // Handed over now, filled in as it is decoded: the file can be used
  // meanwhile, its sound arriving behind it (`decoding` until it all has).
  const sound = { blocks: [], frames: 0, channels: Math.min(2, config.numberOfChannels || 2), decoding: true };
  // Any empty edit before the sound: silence first.
  const lead = Math.round(samples.delay * SOUND_RATE);
  if (lead > 0) {
    sound.blocks.push({ start: 0, planes: Array.from({ length: sound.channels }, () => new Int16Array(lead)) });
    sound.frames = lead;
  }
  entry.sound = sound;
  entry.soundKnown?.();
  // Source samples are counted from the edit list's start: the encoder's
  // priming before it is left out. Decoded and not yet brought to 48 kHz:
  // `pending`, one list of planes a channel, from source sample `base`.
  let pending = null;
  let pendingLength = 0;
  let base = 0;
  let queued = 0; // source samples given to a block so far
  let rate = 0;
  let skip = 0;
  let failed = null;
  let work = Promise.resolve();

  // Source samples [from, to), one Float32Array a channel; silence past
  // what there is.
  const take = (from, to) =>
    pending.map((chunks) => {
      const out = new Float32Array(to - from);
      let at = base;
      for (const chunk of chunks) {
        const lo = Math.max(from, at);
        const hi = Math.min(to, at + chunk.length);
        if (hi > lo) out.set(chunk.subarray(lo - at, hi - at), lo - from);
        at += chunk.length;
        if (at >= to) break;
      }
      return out;
    });
  // Let go of source samples before `before`.
  const drop = (before) => {
    pending = pending.map((chunks) => {
      let at = base;
      const kept = [];
      for (const chunk of chunks) {
        if (at + chunk.length > before) kept.push(at >= before ? chunk : chunk.subarray(before - at));
        at += chunk.length;
      }
      return kept;
    });
    pendingLength -= before - base;
    base = before;
  };
  // Source samples [from, to) brought to 48 kHz and kept, read with padding
  // either side so the blocks join without a seam.
  const block = async (from, to, last) => {
    const inFrom = Math.max(base, from - RESAMPLE_PAD);
    const inTo = last ? to : to + RESAMPLE_PAD;
    const resampled = await to48k(take(inFrom, inTo), rate);
    const ratio = SOUND_RATE / rate;
    const outFrom = Math.round(from * ratio);
    const length = Math.round(to * ratio) - outFrom;
    const offset = Math.round((from - inFrom) * ratio);
    const planes = resampled.map((plane) => {
      const out = new Int16Array(length);
      out.set(toInt16(plane.subarray(offset, offset + length)));
      return out;
    });
    sound.blocks.push({ start: lead + outFrom, planes });
    sound.frames = lead + outFrom + length;
    drop(Math.max(base, to - RESAMPLE_PAD));
  };

  const decoder = new AudioDecoder({
    output: (data) => {
      try {
        if (!rate) {
          rate = data.sampleRate;
          pending = Array.from({ length: sound.channels }, () => []);
          skip = Math.round((samples.start / samples.timescale) * rate);
        }
        const frames = data.numberOfFrames;
        const from = Math.min(skip, frames);
        skip -= from;
        if (from < frames) {
          for (let c = 0; c < sound.channels; c++) {
            const plane = new Float32Array(frames);
            // Mono said to be stereo: the one channel on both sides.
            data.copyTo(plane, { planeIndex: Math.min(c, data.numberOfChannels - 1), format: 'f32-planar' });
            pending[c].push(from ? plane.subarray(from) : plane);
          }
          pendingLength += frames - from;
        }
        const blockLength = SOUND_BLOCK_SECONDS * rate;
        while (base + pendingLength >= queued + blockLength + RESAMPLE_PAD) {
          const [blockFrom, blockTo] = [queued, queued + blockLength];
          queued = blockTo;
          work = work.then(() => block(blockFrom, blockTo, false));
        }
      } catch (err) {
        failed = err;
      } finally {
        data.close();
      }
    },
    error: (err) => {
      failed = err;
    },
  });
  decoder.configure(config);

  // Fed in file order, a run of neighbouring samples read at once.
  let i = 0;
  while (i < samples.count && !failed) {
    let j = i + 1;
    while (
      j < samples.count &&
      samples.offset[j] === samples.offset[j - 1] + samples.size[j - 1] &&
      samples.offset[j] + samples.size[j] - samples.offset[i] <= SOUND_READ_BYTES
    ) {
      j++;
    }
    const bytes = new Uint8Array(
      await entry.file.slice(samples.offset[i], samples.offset[j - 1] + samples.size[j - 1]).arrayBuffer(),
    );
    for (let k = i; k < j; k++) {
      const at = samples.offset[k] - samples.offset[i];
      decoder.decode(
        new EncodedAudioChunk({
          type: 'key',
          timestamp: Math.round((samples.dts[k] / samples.timescale) * 1e6),
          duration: Math.round((samples.dur[k] / samples.timescale) * 1e6),
          data: bytes.subarray(at, at + samples.size[k]),
        }),
      );
    }
    i = j;
    // Not too far ahead of the decoder, nor of the blocks still to make.
    while (decoder.decodeQueueSize > 64) await new Promise((r) => setTimeout(r, 1));
    await work;
  }
  await decoder.flush().catch((err) => {
    failed ??= err;
  });
  decoder.close();
  await work;
  if (failed || !rate) return false;
  if (base + pendingLength > queued) await block(queued, base + pendingLength, true);
  // The encoder's padding after the end, past what the edit list plays, left out.
  if (samples.span > 0) sound.frames = Math.min(sound.frames, lead + Math.round(samples.span * SOUND_RATE));
  entry.sound = sound.channels && sound.frames ? sound : 'none';
  return true;
}

// The file's sound at 48 kHz: `entry.sound` is { blocks, frames, channels },
// each block { start, planes } — one plane a channel, Float32Array as the
// browser decoded it or Int16Array as kept from a long file — or 'none'.

// One sample of a plane, as a float.
const sampleOf = (plane, i) => (plane instanceof Int16Array ? plane[i] / 32767 : plane[i]);

// Min and max of each bucket, interleaved, worked out as the desktop's
// waveform job does: mono mixdown, scaled by 127 and cut toward zero.
function peaksOf(sound) {
  const perBucket = Math.max(1, Math.floor(SOUND_RATE / PEAKS_PER_SECOND));
  const buckets = Math.ceil(sound.frames / perBucket);
  const peaks = new Int8Array(buckets * 2);
  const share = 1 / sound.channels;
  for (const { start, planes } of sound.blocks) {
    const length = planes[0].length;
    for (let i = 0; i < length; i++) {
      let sum = 0;
      for (const plane of planes) sum += sampleOf(plane, i);
      const scaled = Math.trunc(Math.max(-1, Math.min(1, sum * share)) * 127);
      const b = Math.floor((start + i) / perBucket);
      if (scaled < peaks[2 * b]) peaks[2 * b] = scaled;
      if (scaled > peaks[2 * b + 1]) peaks[2 * b + 1] = scaled;
    }
  }
  return peaks;
}

// The whole file given to the browser's decodeAudioData: for what is not an
// MP4 or MOV, or whose sound decodeSoundTrack cannot decode.
function decodeWhole(entry) {
  if (entry.file.size > MAX_SOUND_FILE) {
    entry.sound = 'none';
    return Promise.resolve();
  }
  return entry.file
    .arrayBuffer()
    .then((bytes) => new OfflineAudioContext(2, 1, SOUND_RATE).decodeAudioData(bytes))
    .then((buffer) => {
      const planes = [];
      for (let c = 0; c < Math.min(2, buffer.numberOfChannels); c++) planes.push(buffer.getChannelData(c));
      entry.sound = planes.length
        ? { blocks: [{ start: 0, planes }], frames: buffer.length, channels: planes.length }
        : 'none';
    });
}

// The file's sound decoded. Resolves once it is known well enough to hand
// the file over — whether there is any, and how many channels — which for an
// MP4 is before it has all been decoded: the rest arrives as the file is used.
function decodeSound(entry) {
  const known = new Promise((resolve) => {
    entry.soundKnown = resolve;
  });
  decodeSoundTrack(entry)
    .catch(() => false)
    .then((done) => (done ? undefined : decodeWhole(entry)))
    .then(() => {
      if (entry.sound && entry.sound !== 'none') entry.peaks = peaksOf(entry.sound);
    })
    .catch(() => {
      entry.sound = 'none';
    })
    .finally(() => {
      if (entry.sound && entry.sound !== 'none') entry.sound.decoding = false;
      entry.soundKnown();
      changes += 1;
    });
  return known;
}

function openVideo(entry) {
  const sound = decodeSound(entry);
  const rate = frameRateOf(entry.file).catch(() => null);
  const video = document.createElement('video');
  video.muted = true;
  video.preload = 'auto';
  video.playsInline = true;
  entry.video = video;
  entry.frames = []; // { ms, frame }, by when they were decoded
  video.addEventListener(
    'loadedmetadata',
    () => {
      const hasPicture = video.videoWidth > 0 && video.videoHeight > 0;
      // Handed over once the sound is known too: whether there is any is
      // part of what the editor is told.
      Promise.all([sound, rate]).then(([, rate]) => {
        if (rate) entry.frameMs = (1000 * rate.den) / rate.num;
        finish(entry, {
          rateNum: rate ? rate.num : 0,
          rateDen: rate ? rate.den : 0,
          kind: hasPicture && entry.kind !== 'audio' ? 'video' : 'audio',
          width: video.videoWidth,
          height: video.videoHeight,
          duration: Number.isFinite(video.duration) ? video.duration : 0,
          channels: entry.sound === 'none' ? 0 : entry.sound.channels,
        });
        if (entry.details.kind === 'video' && entry.details.duration > 0) makePictures(entry);
      });
    },
    { once: true },
  );
  video.addEventListener(
    'error',
    () => {
      if (!entry.details) {
        const code = video.error ? video.error.code : 0;
        finish(entry, {
          error: code === 4 ? 'this browser cannot play this kind of file' : 'could not be read',
        });
      }
    },
    { once: true },
  );
  video.addEventListener('seeked', () => {
    if (entry.playing) return;
    const ms = entry.seeking;
    entry.seeking = null;
    if (ms !== null && video.videoWidth > 0) {
      grabLater(video).then((picture) => picture && keep(entry, ms, picture));
    }
    // Asked for somewhere else meanwhile: go there next.
    if (!entry.playing && entry.wanted !== null && entry.wanted !== ms) seek(entry, entry.wanted);
  });
  video.src = entry.url;
}

function seek(entry, ms) {
  entry.seeking = ms;
  entry.video.currentTime = ms / 1000;
}

// Play from `ms`, catching each frame as it is shown.
function play(entry, ms) {
  const video = entry.video;
  entry.playing = true;
  entry.seeking = null;
  video.currentTime = ms / 1000;
  const caught = (_now, metadata) => {
    if (!entry.playing) return;
    const ms = Math.round(metadata.mediaTime * 1000);
    grabLater(video).then((picture) => picture && keep(entry, ms, picture));
    video.requestVideoFrameCallback(caught);
  };
  video.requestVideoFrameCallback(caught);
  video.play().catch(() => {
    entry.playing = false;
  });
  clearInterval(entry.idle);
  entry.idle = setInterval(() => {
    if (performance.now() - entry.askedAt > IDLE_MS) stop(entry);
  }, IDLE_MS / 2);
}

function stop(entry) {
  clearInterval(entry.idle);
  entry.playing = false;
  entry.video.pause();
}

// The frame covering `ms`: one decoded at exactly that time (a seek), or one
// caught while playing that is on screen then.
function frameAt(entry, ms) {
  let best = null;
  for (const f of entry.frames) {
    if (f.ms === ms) return f;
    if (f.ms <= ms && ms - f.ms < (entry.frameMs ?? FRAME_MS) && (!best || f.ms > best.ms)) best = f;
  }
  return best;
}

// ---- what the page calls --------------------------------------------------

// A file dropped on the page or picked: open it, and hand it to the app once
// its details are known. Returns the name the app knows it by. A file kept
// from last time comes with its name, and is not handed over again: the
// project it is in already has it.
function add(file, keptAs, quiet = false) {
  const kept = keptAs !== undefined;
  const id = kept ? Number(keptAs.split('/')[2]) : nextId;
  nextId = Math.max(nextId, id + 1);
  const path = kept ? keptAs : `/browser/${id}/${file.name}`;
  const entry = {
    path,
    file,
    kind: kindOf(file),
    url: URL.createObjectURL(file),
    details: null,
    seeking: null,
    wanted: null,
    last: null,
    still: null,
    poster: null,
    strip: null,
    sound: null,
    peaks: null,
    playing: false,
    askedMs: null,
    askedAt: 0,
    idle: null,
  };
  files.set(path, entry);
  // A file the app made itself (a bounce) is not handed over to import: the
  // app places it once it is open.
  if (!kept && !quiet) order.push(entry);
  pending += 1;
  if (entry.kind === 'image') openImage(entry);
  else openVideo(entry);
  return path;
}

// ---- kept between visits ---------------------------------------------------

let database = null;
let storedProject = null;
const stored = new Set(); // paths of the files kept
// Every project kept, by its id: { json, name, modified, paths } — `paths`
// the kept files it uses. One is the current one, opened at the start.
const projects = new Map();
let currentId = null;
// Kept files not opened yet: those of projects not open. Opened when a
// project that uses them is.
const unopened = new Map(); // path -> File

function mediaPathsOf(json) {
  try {
    const media = JSON.parse(json).project?.media ?? [];
    return new Set(media.map((m) => m.path).filter((path) => typeof path === 'string' && path.startsWith('/browser/')));
  } catch (_) {
    return new Set();
  }
}

// Open the kept files a project uses, as it is opened.
function openKeptFiles(json) {
  for (const path of mediaPathsOf(json)) {
    const file = unopened.get(path);
    if (file && !files.has(path)) {
      unopened.delete(path);
      add(file, path);
    }
  }
}

function request(r) {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

function openDatabase() {
  const opening = indexedDB.open(DATABASE, 2);
  opening.onupgradeneeded = () => {
    const db = opening.result;
    for (const store of [FILES, PROJECTS, KEPT_FILES]) {
      if (!db.objectStoreNames.contains(store)) db.createObjectStore(store);
    }
  };
  return new Promise((resolve, reject) => {
    // A tab still open on an older bettercut holds the database at its
    // older version, and the upgrade waits on it for as long as it stays
    // open. Rather than wait — the page would say "Loading" for ever — the
    // app starts without what was kept (the console says why).
    const waited = setTimeout(
      () => reject(new Error('another bettercut tab is holding the kept projects; close it and reload')),
      4000,
    );
    opening.onsuccess = () => {
      clearTimeout(waited);
      const db = opening.result;
      // And this tab gives way to a newer bettercut opened beside it.
      db.onversionchange = () => db.close();
      resolve(db);
    };
    opening.onerror = () => {
      clearTimeout(waited);
      reject(opening.error);
    };
  });
}

// The editor's own small files (`bettercut_foundation::files`), read whole
// before the app starts so it can read them without waiting, and kept in
// the database as they are written.
const keptFiles = new Map(); // path -> Uint8Array

// Before the app starts: every project kept, and the files of the one open
// last time, opened again under the names it knows them by.
async function restore() {
  try {
    database = await openDatabase();
    const read = database.transaction([FILES, PROJECTS, KEPT_FILES]);
    const [ids, values, paths, kept, keptPaths, keptBytes] = await Promise.all([
      request(read.objectStore(PROJECTS).getAllKeys()),
      request(read.objectStore(PROJECTS).getAll()),
      request(read.objectStore(FILES).getAllKeys()),
      request(read.objectStore(FILES).getAll()),
      request(read.objectStore(KEPT_FILES).getAllKeys()),
      request(read.objectStore(KEPT_FILES).getAll()),
    ]);
    keptPaths.forEach((path, i) => keptFiles.set(path, keptBytes[i]));
    ids.forEach((id, i) => {
      const value = values[i];
      if (id === 'current') {
        // Before there were several, the project itself was kept here.
        if (typeof value === 'string' && value.startsWith('{')) {
          const legacy = keptEntry(value, 0);
          if (legacy) {
            projects.set(legacy.id, legacy);
            currentId = legacy.id;
          }
        } else if (typeof value === 'string') {
          currentId ??= value;
        }
      } else if (value && typeof value.json === 'string') {
        projects.set(id, { ...value, paths: mediaPathsOf(value.json) });
      }
    });
    storedProject = projects.get(currentId)?.json ?? null;
    paths.forEach((path, i) => {
      stored.add(path);
      unopened.set(path, kept[i]);
      // New files are numbered past every kept one, opened or not.
      nextId = Math.max(nextId, Number(String(path).split('/')[2]) + 1 || 0);
    });
    if (storedProject) openKeptFiles(storedProject);
    measureStorage();
  } catch (err) {
    console.warn('bettercut: nothing kept in this browser could be read', err);
  }
}

// A project's JSON as kept: { id, json, name, modified, paths }, or null.
function keptEntry(json, modified) {
  try {
    const project = JSON.parse(json).project;
    if (!project?.id) return null;
    return { id: String(project.id), json, name: String(project.name ?? 'Untitled'), modified, paths: mediaPathsOf(json) };
  } catch (_) {
    return null;
  }
}

// The project as it is now, and the files it uses, kept beside every other
// project kept; a file no kept project uses any more is let go.
function keepProject(json, paths) {
  if (!database) return;
  const entry = keptEntry(json, Date.now());
  if (!entry) return;
  entry.paths = new Set(paths);
  projects.set(entry.id, entry);
  currentId = entry.id;
  const wanted = new Set();
  for (const project of projects.values()) for (const path of project.paths) wanted.add(path);
  const write = database.transaction([FILES, PROJECTS], 'readwrite');
  write.objectStore(PROJECTS).put({ json, name: entry.name, modified: entry.modified }, entry.id);
  write.objectStore(PROJECTS).put(entry.id, 'current');
  for (const path of entry.paths) {
    const entry = files.get(path);
    if (!stored.has(path) && entry) {
      write.objectStore(FILES).put(entry.file, path);
      stored.add(path);
    }
  }
  for (const path of [...stored]) {
    if (!wanted.has(path)) {
      write.objectStore(FILES).delete(path);
      stored.delete(path);
      unopened.delete(path);
    }
  }
  write.oncomplete = measureStorage;
  // Asked once: kept through the browser clearing space, if it agrees.
  if (navigator.storage?.persist && !keepProject.asked) {
    keepProject.asked = true;
    navigator.storage.persist();
  }
}

// A kept project taken out of this browser, with every kept file no other
// project uses. Not the one open: it would only be kept again.
function forgetProject(id) {
  if (!database || id === currentId || !projects.has(id)) return false;
  projects.delete(id);
  const wanted = new Set();
  for (const project of projects.values()) for (const path of project.paths) wanted.add(path);
  const write = database.transaction([FILES, PROJECTS], 'readwrite');
  write.objectStore(PROJECTS).delete(id);
  for (const path of [...stored]) {
    if (!wanted.has(path)) {
      write.objectStore(FILES).delete(path);
      stored.delete(path);
      unopened.delete(path);
    }
  }
  write.oncomplete = measureStorage;
  return true;
}

// How much of the browser's room this page uses, in bytes; -1 until known.
let storageUsed = -1;
function measureStorage() {
  navigator.storage
    ?.estimate?.()
    .then((estimate) => {
      storageUsed = estimate.usage ?? -1;
    })
    .catch(() => {});
}

// Where the last files were dropped, in page pixels, or null when they came
// from the picker.
let dropPoint = null;

// Project files dropped or picked: read, and handed to the app to open under
// a name of their own (`/opened/<n>/<name>`) — not media to import.
const opened = [];
const openedText = new Map();

function isProject(file) {
  return file.name.toLowerCase().endsWith('.vproj');
}

function openProject(file) {
  pending += 1;
  file
    .text()
    .then((text) => {
      // Files given from now on are numbered past the ones the project
      // names, so none takes the name of one it is missing.
      for (const [, n] of text.matchAll(/"\/browser\/(\d+)\//g)) nextId = Math.max(nextId, Number(n) + 1);
      const path = `/opened/${nextId++}/${file.name}`;
      openedText.set(path, text);
      opened.push(path);
    })
    .catch((err) => console.warn('bettercut: the project file could not be read', err))
    .finally(() => {
      pending -= 1;
      changes += 1;
    });
}

function given(files) {
  for (const file of files) {
    if (isProject(file)) openProject(file);
    else add(file);
  }
}

// Files picked for what they say rather than as media — captions, markers:
// their bytes, read, and handed to the app with what they were picked for.
const picked = [];

function pickBytes(accept, purpose) {
  const input = document.createElement('input');
  input.type = 'file';
  if (accept) input.accept = accept;
  input.addEventListener('change', () => {
    const file = input.files[0];
    if (!file) return;
    pending += 1;
    file
      .arrayBuffer()
      .then((bytes) => picked.push([purpose, file.name, new Uint8Array(bytes)]))
      .catch((err) => console.warn('bettercut: the file could not be read', err))
      .finally(() => {
        pending -= 1;
        changes += 1;
      });
  });
  input.click();
}

function pickFiles(accept, multiple) {
  const input = document.createElement('input');
  input.type = 'file';
  input.multiple = multiple;
  if (accept) input.accept = accept;
  input.addEventListener('change', () => {
    dropPoint = null;
    given(input.files);
  });
  input.click();
}

globalThis.bettercutMedia = {
  add,
  restore,
  dropped(files, x, y) {
    dropPoint = { x, y };
    given(files);
  },
  // For testing: how many frames have been decoded so far.
  get changes() {
    return changes;
  },
};

// ---- what the app calls (src/web_media.rs) --------------------------------

export function bc_take_ready() {
  return ready.splice(0, ready.length);
}

export function bc_drop_point() {
  return dropPoint;
}

export function bc_changes() {
  return changes;
}

// Whether any file's sound is still being decoded: an export waits for it.
export function bc_sound_coming() {
  for (const entry of files.values()) if (entry.sound?.decoding) return true;
  return false;
}

export function bc_busy() {
  if (pending > 0 || copying.size > 0) return true;
  for (const entry of files.values()) {
    if (entry.seeking !== null || entry.playing) return true;
  }
  return false;
}

// The project kept last time, as JSON, or null.
export function bc_stored_project() {
  return storedProject;
}

export function bc_keep_project(json, paths) {
  keepProject(json, paths);
}

// The editor's small files (settings, looks, presets), kept in this
// browser's local storage under their paths; null when there is none, or
// storage is closed to this page.
export function bc_setting(path) {
  try {
    return localStorage.getItem('bettercut:' + path);
  } catch (_) {
    return null;
  }
}

export function bc_keep_setting(path, text) {
  try {
    localStorage.setItem('bettercut:' + path, text);
    return true;
  } catch (_) {
    return false;
  }
}

export function bc_kept_file(path) {
  return keptFiles.get(path) ?? undefined;
}

export function bc_kept_list(prefix) {
  return [...keptFiles.keys()].filter((path) => path.startsWith(prefix));
}

export function bc_keep_file(path, bytes) {
  const copy = new Uint8Array(bytes);
  keptFiles.set(path, copy);
  if (!database) return false;
  database.transaction(KEPT_FILES, 'readwrite').objectStore(KEPT_FILES).put(copy, path);
  return true;
}

// A file the app made — a bounced lane's mix — given to the page as if it had
// been dropped, but not handed back to import. Returns the name the app
// knows it by.
export function bc_give_file(bytes, name, type) {
  return add(new File([bytes], name, { type }), undefined, true);
}

// Whether a file the page was given is open: its details known.
export function bc_is_open(path) {
  return Boolean(files.get(path)?.details);
}

export function bc_pick_bytes(accept, purpose) {
  pickBytes(accept, purpose);
}

// Files picked for a purpose since the last look: [purpose, name, bytes].
export function bc_take_given() {
  return picked.splice(0, picked.length);
}

export function bc_pick_files(accept, multiple) {
  pickFiles(accept, multiple);
}

// Project files given to the page since the last look, by the names the
// app opens them under.
export function bc_take_opened() {
  return opened.splice(0, opened.length);
}

// An opened project file's text, once: it is let go of when taken. A
// project kept in this browser (`/kept/<id>/<name>`) is read from there.
// Either way, the kept files it uses are opened.
export function bc_opened_text(path) {
  const text = path.startsWith('/kept/')
    ? (projects.get(path.split('/')[2])?.json ?? null)
    : (openedText.get(path) ?? null);
  openedText.delete(path);
  if (text) openKeptFiles(text);
  return text;
}

export function bc_forget_project(id) {
  return forgetProject(id);
}

export function bc_storage_used() {
  return storageUsed;
}

// Every project kept in this browser, the latest worked on first, by the
// names the app opens them under: its name and when it was last worked on,
// so two called "Untitled" can be told apart.
export function bc_kept_projects() {
  const when = (ms) =>
    ms ? new Date(ms).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : 'earlier';
  const plain = (text) => text.replace(/[\\/]/g, '-');
  return [...projects.entries()]
    .sort(([, a], [, b]) => b.modified - a.modified)
    .map(([id, p]) => `/kept/${id}/${plain(p.name)} · ${plain(when(p.modified))}.vproj`);
}

// Whether the page has a file by this name, and how big it is.
export function bc_file_size(path) {
  return files.get(path)?.file.size ?? -1;
}

export function bc_probe(path) {
  const entry = files.get(path);
  if (!entry || !entry.details) return undefined;
  return { ...entry.details, size: entry.file.size, name: entry.file.name };
}

// `frames` samples of sound from sample `from`, as one Float32Array per
// channel; null past the end, or for a file with no sound.
export function bc_sound(path, from, frames) {
  const sound = files.get(path)?.sound;
  if (!sound || sound === 'none' || from >= sound.frames) return null;
  const end = Math.min(sound.frames, from + frames);
  const out = Array.from({ length: sound.channels }, () => new Float32Array(end - from));
  for (const { start, planes } of sound.blocks) {
    const lo = Math.max(from, start);
    const hi = Math.min(end, start + planes[0].length);
    if (hi <= lo) continue;
    planes.forEach((plane, c) => {
      if (plane instanceof Int16Array) {
        for (let i = lo; i < hi; i++) out[c][i - from] = plane[i - start] / 32767;
      } else {
        out[c].set(plane.subarray(lo - start, hi - start), lo - from);
      }
    });
  }
  return out;
}

// The waveform, min and max of each bucket interleaved, once made.
export function bc_peaks(path) {
  return files.get(path)?.peaks ?? undefined;
}

// The media browser's thumbnail, { width, height, data }, once made.
export function bc_poster(path) {
  return files.get(path)?.poster ?? undefined;
}

// The timeline's filmstrip sheet, { width, height, data }, once made.
export function bc_filmstrip(path) {
  return files.get(path)?.strip ?? undefined;
}

// ---- export's frames, decoded in order --------------------------------------
//
// Seeking a `<video>` to each frame decodes again from the keyframe before it,
// every time. An export wants every frame, in order: so an MP4 or MOV's own
// sample tables say where each frame's bytes are, and the browser's
// VideoDecoder decodes them one after another — several times faster. What
// this cannot read — another container, a codec this browser cannot decode,
// a picture the file turns (a phone held upright) — is sought frame by frame.

// Samples queued in the decoder at most: enough that it never waits for
// more before giving out a frame (an H.264 decoder may hold up to 16).
const DECODE_AHEAD = 20;
// Frames decoded ahead of what was asked for, made ready for the next asks.
const DECODED_AHEAD = 6;
// Not asked for in this long, and the decoder and its frames are let go.
const DECODER_IDLE_MS = 5000;

function bytesOf(view, box) {
  return new Uint8Array(view.buffer, view.byteOffset + box.at, box.end - box.at).slice();
}

const hex2 = (n) => n.toString(16).padStart(2, '0');

// What VideoDecoder needs to be told of a sample entry: { codec,
// description }, or null for a codec not handled here.
function decoderConfigOf(view, entry) {
  // A visual sample entry's own fields take 78 bytes; its boxes follow.
  const config = (type) => boxesIn(view, entry.at + 78, entry.end).find((b) => b.type === type);
  if (entry.type === 'avc1' || entry.type === 'avc3') {
    const c = config('avcC');
    if (!c) return null;
    const [p, k, l] = [1, 2, 3].map((i) => view.getUint8(c.at + i));
    return { codec: `avc1.${hex2(p)}${hex2(k)}${hex2(l)}`, description: bytesOf(view, c) };
  }
  if (entry.type === 'hvc1' || entry.type === 'hev1') {
    const c = config('hvcC');
    if (!c) return null;
    const first = view.getUint8(c.at + 1);
    let flags = view.getUint32(c.at + 2);
    let reversed = 0;
    for (let i = 0; i < 32; i++) {
      reversed = (reversed << 1) | (flags & 1);
      flags >>>= 1;
    }
    const constraints = [];
    for (let i = 0; i < 6; i++) constraints.push(view.getUint8(c.at + 6 + i));
    while (constraints.length && constraints[constraints.length - 1] === 0) constraints.pop();
    const parts = [
      `${entry.type}.${['', 'A', 'B', 'C'][first >> 6]}${first & 0x1f}`,
      (reversed >>> 0).toString(16),
      `${(first >> 5) & 1 ? 'H' : 'L'}${view.getUint8(c.at + 12)}`,
      ...constraints.map((b) => b.toString(16)),
    ];
    return { codec: parts.join('.'), description: bytesOf(view, c) };
  }
  if (entry.type === 'av01') {
    const c = config('av1C');
    if (!c) return null;
    const [a, b] = [1, 2].map((i) => view.getUint8(c.at + i));
    const depth = (b >> 6) & 1 ? ((b >> 5) & 1 ? 12 : 10) : 8;
    const level = String(a & 0x1f).padStart(2, '0');
    return { codec: `av01.${a >> 5}.${level}${b >> 7 ? 'H' : 'M'}.${String(depth).padStart(2, '0')}`, description: bytesOf(view, c) };
  }
  if (entry.type === 'vp09') {
    const c = config('vpcC');
    if (!c) return null;
    const two = (n) => String(n).padStart(2, '0');
    return { codec: `vp09.${two(view.getUint8(c.at + 4))}.${two(view.getUint8(c.at + 5))}.${two(view.getUint8(c.at + 6) >> 4)}` };
  }
  return null;
}

// The movie's tracks of one kind — 'vide' or 'soun' — and its own timescale.
function tracksOf(moov, kind) {
  const mvhd = child(moov, { at: 8, end: moov.byteLength }, 'mvhd');
  const movieScale = mvhd ? moov.getUint32(mvhd.at + (moov.getUint8(mvhd.at) === 1 ? 20 : 12)) : 0;
  const found = [];
  for (const trak of boxesIn(moov, 8, moov.byteLength).filter((b) => b.type === 'trak')) {
    const hdlr = child(moov, child(moov, trak, 'mdia'), 'hdlr');
    if (!hdlr || hdlr.at + 12 > hdlr.end) continue;
    if (String.fromCharCode(...[8, 9, 10, 11].map((i) => moov.getUint8(hdlr.at + i))) === kind) found.push(trak);
  }
  return { tracks: found, movieScale };
}

// A track's sample table: where each sample's bytes are, how long it lasts
// and when it is decoded (in the track's timescale), its composition offset,
// and the edit list's start, any empty edit before it and how long the edit
// plays (seconds; 0 when there is no edit list). null when a table is
// missing.
function sampleTableOf(moov, trak, movieScale) {
  const mdia = child(moov, trak, 'mdia');
  const mdhd = child(moov, mdia, 'mdhd');
  const stbl = child(moov, child(moov, mdia, 'minf'), 'stbl');
  const table = (type) => child(moov, stbl, type);
  const [stsd, stts, stsz, stsc] = ['stsd', 'stts', 'stsz', 'stsc'].map(table);
  const chunks = table('stco') ?? table('co64');
  if (!mdhd || !stsd || !stts || !stsz || !stsc || !chunks) return null;
  const timescale = moov.getUint32(mdhd.at + (moov.getUint8(mdhd.at) === 1 ? 20 : 12));
  const entry = boxesIn(moov, stsd.at + 8, stsd.end)[0];
  if (!timescale || !entry) return null;

  const count = moov.getUint32(stsz.at + 8);
  const sameSize = moov.getUint32(stsz.at + 4);
  const size = new Float64Array(count);
  for (let i = 0; i < count; i++) size[i] = sameSize || moov.getUint32(stsz.at + 12 + i * 4);

  // Decode times, from the run lengths of stts.
  const dts = new Float64Array(count);
  const dur = new Float64Array(count);
  let i = 0;
  let t = 0;
  for (let e = 0, n = moov.getUint32(stts.at + 4); e < n && i < count; e++) {
    const run = moov.getUint32(stts.at + 8 + e * 8);
    const delta = moov.getUint32(stts.at + 12 + e * 8);
    for (let r = 0; r < run && i < count; r++, i++) {
      dts[i] = t;
      dur[i] = delta;
      t += delta;
    }
  }

  const cts = new Float64Array(count);
  const ctts = table('ctts');
  if (ctts) {
    const signed = moov.getUint8(ctts.at) === 1;
    let s = 0;
    for (let e = 0, n = moov.getUint32(ctts.at + 4); e < n && s < count; e++) {
      const run = moov.getUint32(ctts.at + 8 + e * 8);
      const offset = signed ? moov.getInt32(ctts.at + 12 + e * 8) : moov.getUint32(ctts.at + 12 + e * 8);
      for (let r = 0; r < run && s < count; r++, s++) cts[s] = offset;
    }
  }

  let start = 0;
  let delay = 0;
  let span = 0;
  const elst = child(moov, child(moov, trak, 'edts'), 'elst');
  if (elst) {
    const wide = moov.getUint8(elst.at) === 1;
    const step = wide ? 20 : 12;
    for (let e = 0, n = moov.getUint32(elst.at + 4); e < n; e++) {
      const at = elst.at + 8 + e * step;
      if (at + step > elst.end) break;
      const length = wide ? Number(moov.getBigUint64(at)) : moov.getUint32(at);
      const from = wide ? Number(moov.getBigInt64(at + 8)) : moov.getInt32(at + 4);
      if (from === -1) {
        delay += movieScale ? length / movieScale : 0;
        continue;
      }
      start = from;
      span = movieScale ? length / movieScale : 0;
      break;
    }
  }

  // Where each sample's bytes are: chunk by chunk, as many samples a chunk
  // as stsc says.
  const offset = new Float64Array(count);
  const wideChunks = chunks.type === 'co64';
  const chunkCount = moov.getUint32(chunks.at + 4);
  const runs = moov.getUint32(stsc.at + 4);
  let s = 0;
  for (let r = 0; r < runs && s < count; r++) {
    const first = moov.getUint32(stsc.at + 8 + r * 12);
    const perChunk = moov.getUint32(stsc.at + 12 + r * 12);
    const last = r + 1 < runs ? moov.getUint32(stsc.at + 8 + (r + 1) * 12) - 1 : chunkCount;
    for (let c = first; c <= last && s < count; c++) {
      let at = wideChunks
        ? Number(moov.getBigUint64(chunks.at + 8 + (c - 1) * 8))
        : moov.getUint32(chunks.at + 8 + (c - 1) * 4);
      for (let k = 0; k < perChunk && s < count; k++, s++) {
        offset[s] = at;
        at += size[s];
      }
    }
  }
  return { timescale, entry, count, offset, size, dts, dur, cts, start, delay, span, table };
}

// The video track's samples, from an MP4 or MOV's tables: where each one's
// bytes are, when it is shown (microseconds, as the `<video>` shows it —
// edit list applied), and whether it is a keyframe. null when there is no
// such track this can decode.
async function videoSamplesOf(file) {
  const moov = await moovOf(file);
  if (!moov) return null;
  const { tracks, movieScale } = tracksOf(moov, 'vide');
  for (const trak of tracks) {
    // A picture the file turns or flips is shown turned by a `<video>`, not
    // by a decoder: left to the `<video>`.
    const tkhd = child(moov, trak, 'tkhd');
    if (tkhd) {
      const m = tkhd.at + (moov.getUint8(tkhd.at) === 1 ? 52 : 40);
      const [a, b, c, d] = [0, 4, 12, 16].map((i) => moov.getInt32(m + i));
      if (a !== 0x10000 || b !== 0 || c !== 0 || d !== 0x10000) return null;
    }
    const samples = sampleTableOf(moov, trak, movieScale);
    if (!samples) return null;
    const { timescale, entry, count, offset, size, dts, cts, start, delay } = samples;
    const config = decoderConfigOf(moov, entry);
    if (!config) return null;

    // Shown at decode time plus the composition offset, less where the edit
    // list starts, plus any empty edit before it.
    const time = new Float64Array(count);
    const dur = new Float64Array(count);
    const byTime = new Map();
    for (let s = 0; s < count; s++) {
      time[s] = Math.round(((dts[s] + cts[s] - start) / timescale + delay) * 1e6);
      byTime.set(time[s], s);
      dur[s] = Math.round((samples.dur[s] / timescale) * 1e6);
    }

    const key = new Uint8Array(count);
    const stss = samples.table('stss');
    if (stss) {
      for (let e = 0, n = moov.getUint32(stss.at + 4); e < n; e++) {
        const sample = moov.getUint32(stss.at + 8 + e * 4) - 1;
        if (sample < count) key[sample] = 1;
      }
    } else {
      key.fill(1);
    }
    const keys = [];
    for (let k = 0; k < count; k++) if (key[k]) keys.push(k);
    const shown = [...Array(count).keys()].sort((a, b) => time[a] - time[b]);
    if (!count || !keys.length || keys[0] !== 0) return null;
    return {
      config: {
        ...config,
        codedWidth: moov.getUint16(entry.at + 24),
        codedHeight: moov.getUint16(entry.at + 26),
        // Each frame out as soon as it can be, not when the decoder likes.
        optimizeForLatency: true,
      },
      count, offset, size, time, dur, key, keys, byTime, shown,
    };
  }
  return null;
}

// The last sample shown at or before `us`, or the first.
function sampleAt(track, us) {
  let lo = 0;
  let hi = track.shown.length - 1;
  let found = track.shown[0];
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (track.time[track.shown[mid]] <= us) {
      found = track.shown[mid];
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
}

// The keyframe decoding has to start from to reach sample `k`.
function keyBefore(track, k) {
  let lo = 0;
  let hi = track.keys.length - 1;
  let found = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (track.keys[mid] <= k) {
      found = track.keys[mid];
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
}

// A decoded frame as RGBA, sRGB, at its display size; the frame is closed.
async function pictureOf(frame) {
  const width = frame.displayWidth;
  const height = frame.displayHeight;
  try {
    if (frame.visibleRect && frame.visibleRect.width === width && frame.visibleRect.height === height) {
      try {
        const data = new Uint8ClampedArray(width * height * 4);
        await frame.copyTo(data, { format: 'RGBA', colorSpace: 'srgb' });
        return { width, height, data };
      } catch (_) {
        // This browser cannot convert as it copies: drawn instead.
      }
    }
    const canvas = new OffscreenCanvas(width, height);
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.drawImage(frame, 0, 0, width, height);
    return { width, height, data: context.getImageData(0, 0, width, height).data };
  } finally {
    frame.close();
  }
}

function openDecoding(entry) {
  const d = {
    state: 'opening', // then 'ready', or 'none': seek instead
    track: null,
    decoder: null,
    epoch: 0,
    next: 0, // the next sample to give the decoder, in decode order
    lastOut: -Infinity, // the time of the last frame out since a restart
    reading: false,
    flushed: false,
    wanted: [], // samples asked for, not yet had
    askedTime: -Infinity, // when the sample last asked for is shown
    converting: new Set(),
    pictures: new Map(), // sample -> picture, the last few asked for
    askedAt: performance.now(),
    idle: null,
  };
  (async () => {
    if (typeof VideoDecoder === 'undefined') throw new Error('no VideoDecoder');
    const track = await videoSamplesOf(entry.file);
    if (!track) throw new Error('not a track this decodes');
    const support = await VideoDecoder.isConfigSupported(track.config);
    if (!support.supported) throw new Error(`cannot decode ${track.config.codec}`);
    d.track = track;
    d.file = entry.file;
    restart(d, 0);
    d.state = 'ready';
    d.idle = setInterval(() => {
      if (performance.now() - d.askedAt > DECODER_IDLE_MS) {
        closeDecoding(d);
        if (entry.decoding === d) entry.decoding = undefined;
      }
    }, DECODER_IDLE_MS / 2);
    pump(d);
  })().catch(() => {
    closeDecoding(d);
  });
  return d;
}

function closeDecoding(d) {
  d.state = 'none';
  clearInterval(d.idle);
  try {
    d.decoder?.close();
  } catch (_) {
    // Closed already.
  }
}

// Decode again from sample `from`, a keyframe.
function restart(d, from) {
  d.epoch += 1;
  d.reading = false;
  d.flushed = false;
  d.next = from;
  d.lastOut = -Infinity;
  try {
    d.decoder?.close();
  } catch (_) {
    // Closed already.
  }
  d.decoder = new VideoDecoder({
    output: (frame) => decoded(d, frame),
    error: () => closeDecoding(d),
  });
  d.decoder.configure(d.track.config);
}

function convert(d, k, frame) {
  d.converting.add(k);
  pictureOf(frame)
    .then((picture) => {
      d.converting.delete(k);
      d.pictures.set(k, picture);
      while (d.pictures.size > EXACT_KEPT + DECODED_AHEAD) d.pictures.delete(d.pictures.keys().next().value);
      changes += 1;
    })
    .catch(() => closeDecoding(d));
}

function decoded(d, frame) {
  const k = d.track.byTime.get(frame.timestamp);
  d.lastOut = Math.max(d.lastOut, frame.timestamp);
  if (k === undefined) {
    frame.close();
  } else if (d.wanted.includes(k)) {
    d.wanted = d.wanted.filter((w) => w !== k);
    convert(d, k, frame);
  } else if (d.track.time[k] > d.askedTime && !d.pictures.has(k) && !d.converting.has(k)) {
    // Likely the next asked for: made ready now.
    convert(d, k, frame);
  } else {
    frame.close();
  }
  pump(d);
}

// Frames made ready, or being made, past the last one asked for.
function ahead(d) {
  let n = 0;
  for (const k of d.pictures.keys()) if (d.track.time[k] > d.askedTime) n++;
  for (const k of d.converting) if (d.track.time[k] > d.askedTime) n++;
  return n;
}

// Give the decoder what it needs next: from a keyframe again when what is
// wanted has gone by or lies past the next keyframe, and onward otherwise.
function pump(d) {
  if (d.state !== 'ready') return;
  const track = d.track;
  d.wanted = d.wanted.filter((k) => !d.pictures.has(k) && !d.converting.has(k));
  if (d.wanted.length) {
    const first = d.wanted.reduce((a, b) => (track.time[a] <= track.time[b] ? a : b));
    const key = keyBefore(track, first);
    if (track.time[first] <= d.lastOut || d.next < key) {
      // Gone by again and again: this decoder does not give it, so seek.
      d.restarts = d.restartedFor === first ? d.restarts + 1 : 0;
      d.restartedFor = first;
      if (d.restarts > 2) {
        closeDecoding(d);
        return;
      }
      restart(d, key);
    }
  }
  feed(d);
}

async function feed(d) {
  if (d.reading || d.state !== 'ready') return;
  d.reading = true;
  const epoch = d.epoch;
  const track = d.track;
  try {
    while (
      d.state === 'ready' &&
      d.next < track.count &&
      d.decoder.decodeQueueSize < DECODE_AHEAD &&
      (d.wanted.length > 0 || ahead(d) < DECODED_AHEAD)
    ) {
      const k = d.next++;
      const data = await d.file.slice(track.offset[k], track.offset[k] + track.size[k]).arrayBuffer();
      if (epoch !== d.epoch || d.state !== 'ready') return;
      d.decoder.decode(
        new EncodedVideoChunk({
          type: track.key[k] ? 'key' : 'delta',
          timestamp: track.time[k],
          duration: track.dur[k],
          data,
        }),
      );
    }
    // The last frames come out only once the decoder is told there are no more.
    if (d.state === 'ready' && d.next >= track.count && !d.flushed && d.wanted.length) {
      d.flushed = true;
      d.decoder.flush().catch(() => {});
    }
  } finally {
    if (epoch === d.epoch) d.reading = false;
  }
}

// The frame at `ms` from the decoder, or undefined while it is coming.
function decodedAt(d, ms) {
  d.askedAt = performance.now();
  // Half a millisecond in, as the `<video>` is sought.
  const k = sampleAt(d.track, (ms + 0.5) * 1000);
  d.askedTime = d.track.time[k];
  const picture = d.pictures.get(k);
  if (picture) return picture;
  if (!d.wanted.includes(k) && !d.converting.has(k)) d.wanted.push(k);
  pump(d);
  return undefined;
}

// Export's frames, when they cannot be decoded in order: a `<video>` of
// their own, so an export never moves the one the preview reads from, sought
// to every frame asked for — exact, at full size, never "the last one there
// was".
//
// The same file can be two layers at once (a clip on two tracks, a picture
// in picture), each wanting its own frame: so the last few frames are kept,
// and every frame asked for is fetched in turn. Remembering only one, the
// layers would take it from each other forever and the export never move.
const EXACT_KEPT = 4;
// A seek not answered in this long is asked again.
const EXACT_PATIENCE_MS = 3000;

function exportVideo(entry) {
  const video = document.createElement('video');
  video.muted = true;
  video.preload = 'auto';
  const exporting = { video, ready: false, seeking: null, since: 0, wanted: [], frames: new Map() };
  const go = (ms) => {
    exporting.seeking = ms;
    exporting.since = performance.now();
    // Half a millisecond in, so a frame starting on this very millisecond is
    // the one shown rather than the one before it.
    video.currentTime = (ms + 0.5) / 1000;
  };
  const next = () => {
    exporting.wanted = exporting.wanted.filter((ms) => !exporting.frames.has(ms));
    if (exporting.wanted.length) go(exporting.wanted[0]);
  };
  exporting.go = go;
  exporting.next = next;
  video.addEventListener(
    'loadeddata',
    () => {
      exporting.ready = true;
      next();
    },
    { once: true },
  );
  video.addEventListener('seeked', () => {
    const ms = exporting.seeking;
    exporting.seeking = null;
    if (ms !== null && video.videoWidth > 0) {
      const picture = grabWidth(video, video.videoWidth, video.videoHeight, Infinity);
      if (picture) {
        exporting.frames.delete(ms);
        exporting.frames.set(ms, picture);
        while (exporting.frames.size > EXACT_KEPT) {
          exporting.frames.delete(exporting.frames.keys().next().value);
        }
      }
    }
    next();
  });
  video.src = entry.url;
  return exporting;
}

// The frame at `seconds` exactly, for an export: { width, height, data,
// exact }, or undefined while it is being fetched — ask again.
export function bc_frame_exact(path, seconds) {
  const entry = files.get(path);
  if (!entry) return undefined;
  if (entry.still) return { ...entry.still, exact: true };
  if (!entry.video) return undefined;
  const ms = Math.max(0, Math.round(seconds * 1000));
  // Decoded in order where this can; sought frame by frame where not.
  entry.decoding ??= openDecoding(entry);
  const decoding = entry.decoding;
  if (decoding.state === 'opening') return undefined;
  if (decoding.state === 'ready') {
    const picture = decodedAt(decoding, ms);
    if (picture) return { ...picture, exact: true };
    if (decoding.state === 'ready') return undefined;
  }
  entry.exporting ??= exportVideo(entry);
  const exporting = entry.exporting;
  const kept = exporting.frames.get(ms);
  if (kept) return { ...kept, exact: true };
  if (!exporting.wanted.includes(ms)) exporting.wanted.push(ms);
  if (!exporting.ready) return undefined;
  if (exporting.seeking === null) {
    exporting.next();
  } else if (performance.now() - exporting.since > EXACT_PATIENCE_MS) {
    // The browser dropped this seek: ask again rather than wait forever.
    exporting.go(exporting.seeking);
  }
  return undefined;
}

// The frame at `seconds`: { width, height, data, exact }, or undefined when
// nothing of this file has been decoded yet. Not exact means it is the last
// frame there was, shown while the right one is fetched.
export function bc_frame(path, seconds) {
  const entry = files.get(path);
  if (!entry) return undefined;
  if (entry.still) return { ...entry.still, exact: true };
  if (!entry.video) return undefined;
  const ms = Math.max(0, Math.round(seconds * 1000));

  // A little later than the last request: playback. Anything else: a seek.
  const step = entry.askedMs === null ? -1 : ms - entry.askedMs;
  const playback = step > 0 && step < PLAYBACK_STEP_MS;
  entry.askedMs = ms;
  entry.askedAt = performance.now();

  const hit = frameAt(entry, ms);
  if (hit) return { ...hit.frame, exact: true };

  if (playback) {
    // Fallen too far behind (or ahead) of what is playing: start again here.
    const playingAt = entry.video.currentTime * 1000;
    if (!entry.playing || Math.abs(playingAt - ms) > 500) play(entry, ms);
  } else {
    if (entry.playing) stop(entry);
    entry.wanted = ms;
    if (entry.seeking === null) seek(entry, ms);
  }
  return entry.last ? { ...entry.last.frame, exact: false } : undefined;
}

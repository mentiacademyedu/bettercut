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
const PROJECTS = 'projects'; // 'current' -> the project's JSON

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
  // Find the movie header among the file's top-level boxes.
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

// The file's sound, decoded by the browser into planes of 48 kHz samples:
// `entry.sound` is { planes, frames }, or 'none'.
// Min and max of each bucket, interleaved, worked out as the desktop's
// waveform job does: mono mixdown, scaled by 127 and cut toward zero.
function peaksOf(planes, frames) {
  const perBucket = Math.max(1, Math.floor(SOUND_RATE / PEAKS_PER_SECOND));
  const buckets = Math.ceil(frames / perBucket);
  const peaks = new Int8Array(buckets * 2);
  const share = 1 / planes.length;
  for (let b = 0; b < buckets; b++) {
    let low = 0;
    let high = 0;
    const end = Math.min(frames, (b + 1) * perBucket);
    for (let i = b * perBucket; i < end; i++) {
      let sum = 0;
      for (const plane of planes) sum += plane[i];
      const scaled = Math.trunc(Math.max(-1, Math.min(1, sum * share)) * 127);
      if (scaled < low) low = scaled;
      if (scaled > high) high = scaled;
    }
    peaks[2 * b] = low;
    peaks[2 * b + 1] = high;
  }
  return peaks;
}

function decodeSound(entry) {
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
      entry.sound = planes.length ? { planes, frames: buffer.length } : 'none';
      if (planes.length) entry.peaks = peaksOf(planes, buffer.length);
    })
    .catch(() => {
      entry.sound = 'none';
    });
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
          channels: entry.sound === 'none' ? 0 : entry.sound.planes.length,
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
    if (ms !== null && video.videoWidth > 0) keep(entry, ms, grab(video, video.videoWidth, video.videoHeight));
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
    keep(entry, Math.round(metadata.mediaTime * 1000), grab(video, video.videoWidth, video.videoHeight));
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
function add(file, keptAs) {
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
  if (!kept) order.push(entry);
  pending += 1;
  if (entry.kind === 'image') openImage(entry);
  else openVideo(entry);
  return path;
}

// ---- kept between visits ---------------------------------------------------

let database = null;
let storedProject = null;
const stored = new Set(); // paths of the files kept

function request(r) {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

function openDatabase() {
  const opening = indexedDB.open(DATABASE, 1);
  opening.onupgradeneeded = () => {
    opening.result.createObjectStore(FILES);
    opening.result.createObjectStore(PROJECTS);
  };
  return request(opening);
}

// Before the app starts: the project kept last time, and every file it
// uses, opened again under the names it knows them by.
async function restore() {
  try {
    database = await openDatabase();
    const read = database.transaction([FILES, PROJECTS]);
    const [project, paths, kept] = await Promise.all([
      request(read.objectStore(PROJECTS).get('current')),
      request(read.objectStore(FILES).getAllKeys()),
      request(read.objectStore(FILES).getAll()),
    ]);
    storedProject = typeof project === 'string' ? project : null;
    paths.forEach((path, i) => {
      stored.add(path);
      add(kept[i], path);
    });
  } catch (err) {
    console.warn('bettercut: nothing kept in this browser could be read', err);
  }
}

// The project as it is now, and the files it uses: kept, and every other
// file kept before let go.
function keepProject(json, paths) {
  if (!database) return;
  const wanted = new Set(paths);
  const write = database.transaction([FILES, PROJECTS], 'readwrite');
  write.objectStore(PROJECTS).put(json, 'current');
  for (const path of wanted) {
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
    }
  }
  // Asked once: kept through the browser clearing space, if it agrees.
  if (navigator.storage?.persist && !keepProject.asked) {
    keepProject.asked = true;
    navigator.storage.persist();
  }
}

// Where the last files were dropped, in page pixels, or null when they came
// from the picker.
let dropPoint = null;

function pickFiles(accept) {
  const input = document.createElement('input');
  input.type = 'file';
  input.multiple = true;
  if (accept) input.accept = accept;
  input.addEventListener('change', () => {
    dropPoint = null;
    for (const file of input.files) add(file);
  });
  input.click();
}

globalThis.bettercutMedia = {
  add,
  restore,
  dropped(files, x, y) {
    dropPoint = { x, y };
    for (const file of files) add(file);
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

export function bc_busy() {
  if (pending > 0) return true;
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

export function bc_pick_files(accept) {
  pickFiles(accept);
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
  return sound.planes.map((plane) => plane.subarray(from, end));
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

// Export's frames: a `<video>` of their own, so an export never moves the
// one the preview reads from, sought to every frame asked for — exact, at
// full size, never "the last one there was".
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

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
const MAX_SOUND_FILE = 1536 * 1024 * 1024;
// How long one frame is assumed to cover, for matching a request to a frame
// caught while playing: a `<video>` does not say its rate.
const FRAME_MS = 1000 / 30;
// Requests this close after the last, and later in the file, are playback.
const PLAYBACK_STEP_MS = 250;
// Quiet this long while playing, and the video is paused again.
const IDLE_MS = 400;

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
    })
    .catch(() => {
      entry.sound = 'none';
    });
}

function openVideo(entry) {
  const sound = decodeSound(entry);
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
      sound.then(() => {
        finish(entry, {
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
    if (f.ms <= ms && ms - f.ms < FRAME_MS && (!best || f.ms > best.ms)) best = f;
  }
  return best;
}

// ---- what the page calls --------------------------------------------------

// A file dropped on the page or picked: open it, and hand it to the app once
// its details are known. Returns the name the app knows it by.
function add(file) {
  const path = `/browser/${nextId++}/${file.name}`;
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
    playing: false,
    askedMs: null,
    askedAt: 0,
    idle: null,
  };
  files.set(path, entry);
  order.push(entry);
  pending += 1;
  if (entry.kind === 'image') openImage(entry);
  else openVideo(entry);
  return path;
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

// The media browser's thumbnail, { width, height, data }, once made.
export function bc_poster(path) {
  return files.get(path)?.poster ?? undefined;
}

// The timeline's filmstrip sheet, { width, height, data }, once made.
export function bc_filmstrip(path) {
  return files.get(path)?.strip ?? undefined;
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

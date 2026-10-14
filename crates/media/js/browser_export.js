// The web build's export: the browser's own encoders (WebCodecs) turn the
// frames the app renders, and the sound it mixes, into H.264 and AAC — or
// Opus where this browser cannot make AAC. What they make is handed back to
// the app, which writes the MP4 (`src/mp4_mux.rs`), and the finished file is
// given to the person as a download.
//
// One export at a time, driven by the app a frame at a time; everything
// here answers at once and the app asks again next frame.

// H.264 High, level 5.1: enough for 4K at 30 and anything smaller.
const VIDEO_CODECS = ['avc1.640033', 'avc1.4d0033', 'avc1.420033'];
const AAC = 'mp4a.40.2';
const SOUND_RATE = 48000;
const SOUND_BITRATE = 192000;
// Frames queued in the encoder before the app is asked to wait.
const BACKLOG = 6;

let job = null;

function fail(message) {
  if (job && job.state !== 'cancelled') job.state = 'failed: ' + message;
}

async function firstSupported(make, codecs) {
  for (const codec of codecs) {
    const config = make(codec);
    try {
      const answer = await (config.numberOfChannels ? AudioEncoder : VideoEncoder).isConfigSupported(config);
      if (answer.supported) return config;
    } catch (_) {
      // Not this one.
    }
  }
  return null;
}

async function configure(current, width, height, rate, bitrate, channels) {
  if (typeof VideoEncoder === 'undefined') throw new Error('this browser cannot encode video');
  const video = await firstSupported(
    (codec) => ({ codec, width, height, bitrate, framerate: rate, avc: { format: 'avc' } }),
    VIDEO_CODECS,
  );
  if (!video) throw new Error(`this browser cannot encode H.264 at ${width}×${height}`);
  current.video = new VideoEncoder({
    output: (chunk, meta) => {
      if (meta?.decoderConfig?.description && !current.videoDescription) {
        current.videoDescription = new Uint8Array(meta.decoderConfig.description.slice(0));
      }
      const data = new Uint8Array(chunk.byteLength);
      chunk.copyTo(data);
      current.videoChunks.push({ data, key: chunk.type === 'key' });
    },
    error: (e) => fail(String(e.message || e)),
  });
  current.video.configure(video);

  if (channels > 0 && typeof AudioEncoder !== 'undefined') {
    const make = (codec) => ({ codec, sampleRate: SOUND_RATE, numberOfChannels: channels, bitrate: SOUND_BITRATE });
    const sound = await firstSupported(make, [AAC, 'opus']);
    if (sound) {
      current.soundCodec = sound.codec === AAC ? 'aac' : 'opus';
      current.sound = new AudioEncoder({
        output: (chunk, meta) => {
          if (meta?.decoderConfig?.description && !current.soundDescription) {
            current.soundDescription = new Uint8Array(meta.decoderConfig.description.slice(0));
          }
          const data = new Uint8Array(chunk.byteLength);
          chunk.copyTo(data);
          current.soundChunks.push({ data, duration: chunk.duration ?? 0 });
        },
        error: (e) => fail(String(e.message || e)),
      });
      current.sound.configure(sound);
      current.channels = channels;
    }
  }
}

// ---- what the app calls (src/web_media.rs) --------------------------------

export function bc_export_start(width, height, rateNum, rateDen, bitrate, channels) {
  if (job) bc_export_cancel();
  const current = {
    state: 'starting',
    rateNum,
    rateDen,
    video: null,
    sound: null,
    soundCodec: null,
    channels: 0,
    videoDescription: null,
    soundDescription: null,
    videoChunks: [],
    soundChunks: [],
    soundFrames: 0,
  };
  job = current;
  configure(current, width, height, rateNum / rateDen, bitrate, channels)
    .then(() => {
      if (current.state === 'starting') current.state = 'ready';
    })
    .catch((e) => {
      current.state = 'failed: ' + String(e.message || e);
    });
}

// 'starting', 'ready', 'finishing', 'done', 'cancelled' or 'failed: why'.
export function bc_export_state() {
  return job ? job.state : 'none';
}

// Whether the encoder has room for another frame.
export function bc_export_has_room() {
  return !!job && job.state === 'ready' && job.video.encodeQueueSize < BACKLOG;
}

// The sound this export carries: 'aac', 'opus', or null for none.
export function bc_export_sound_codec() {
  return job ? job.soundCodec : null;
}

// Frame `index`, RGBA, `width` × `height`.
// RGBA, as rendered, to I420 in BT.709 limited range — the colours the MP4
// says it holds. Converted here rather than by the encoder: each browser's
// encoder converts its own way, and says nothing of which.
function toI420(rgba, width, height) {
  const cw = (width + 1) >> 1;
  const ch = (height + 1) >> 1;
  const out = new Uint8Array(width * height + 2 * cw * ch);
  const u0 = width * height;
  const v0 = u0 + cw * ch;
  // Fixed point, 16 fractional bits: Y = 16 + 219·(0.2126 R + 0.7152 G +
  // 0.0722 B), and the colour differences scaled to 224, as BT.709 has it.
  const yr = Math.round(65536 * 0.2126 * 219 / 255);
  const yg = Math.round(65536 * 0.7152 * 219 / 255);
  const yb = Math.round(65536 * 0.0722 * 219 / 255);
  const ur = Math.round(65536 * (-0.2126 / 1.8556) * 224 / 255);
  const ug = Math.round(65536 * (-0.7152 / 1.8556) * 224 / 255);
  const ub = Math.round(65536 * (0.9278 / 1.8556) * 224 / 255);
  const vr = Math.round(65536 * (0.7874 / 1.5748) * 224 / 255);
  const vg = Math.round(65536 * (-0.7152 / 1.5748) * 224 / 255);
  const vb = Math.round(65536 * (-0.0722 / 1.5748) * 224 / 255);
  const half = 32768;
  for (let y = 0; y < height; y++) {
    let p = y * width * 4;
    let q = y * width;
    for (let x = 0; x < width; x++, p += 4, q++) {
      out[q] = (16 * 65536 + yr * rgba[p] + yg * rgba[p + 1] + yb * rgba[p + 2] + half) >> 16;
    }
  }
  // Each chroma sample from the average of the 2×2 pixels it covers.
  for (let y = 0; y < ch; y++) {
    const y1 = Math.min(height - 1, 2 * y + 1);
    for (let x = 0; x < cw; x++) {
      const x1 = Math.min(width - 1, 2 * x + 1);
      let r = 0;
      let g = 0;
      let b = 0;
      for (const [px, py] of [[2 * x, 2 * y], [x1, 2 * y], [2 * x, y1], [x1, y1]]) {
        const p = (py * width + px) * 4;
        r += rgba[p];
        g += rgba[p + 1];
        b += rgba[p + 2];
      }
      // The sums are four pixels: a quarter of each coefficient.
      out[u0 + y * cw + x] = (128 * 65536 + ((ur * r + ug * g + ub * b) >> 2) + half) >> 16;
      out[v0 + y * cw + x] = (128 * 65536 + ((vr * r + vg * g + vb * b) >> 2) + half) >> 16;
    }
  }
  return out;
}

export function bc_export_frame(rgba, width, height, index, key) {
  if (!job || job.state !== 'ready') return;
  const micros = (n) => Math.round((n * 1e6 * job.rateDen) / job.rateNum);
  const frame = new VideoFrame(toI420(rgba, width, height), {
    format: 'I420',
    codedWidth: width,
    codedHeight: height,
    timestamp: micros(index),
    duration: micros(index + 1) - micros(index),
    colorSpace: { primaries: 'bt709', transfer: 'bt709', matrix: 'bt709', fullRange: false },
  });
  job.video.encode(frame, { keyFrame: key });
  frame.close();
}

// Sound, one Float32Array a channel, all the same length, following on
// from the last.
export function bc_export_sound(planes) {
  if (!job || job.state !== 'ready' || !job.sound || !planes.length) return;
  const frames = planes[0].length;
  if (!frames) return;
  const data = new Float32Array(frames * job.channels);
  for (let c = 0; c < job.channels; c++) data.set(planes[Math.min(c, planes.length - 1)], c * frames);
  const audio = new AudioData({
    format: 'f32-planar',
    sampleRate: SOUND_RATE,
    numberOfFrames: frames,
    numberOfChannels: job.channels,
    timestamp: Math.round((job.soundFrames * 1e6) / SOUND_RATE),
    data,
  });
  job.soundFrames += frames;
  job.sound.encode(audio);
  audio.close();
}

// Everything given so far, through the encoders; `done` once it is.
export function bc_export_finish() {
  if (!job || job.state !== 'ready') return;
  const current = job;
  current.state = 'finishing';
  Promise.all([current.video.flush(), current.sound ? current.sound.flush() : null])
    .then(() => {
      if (current.state === 'finishing') current.state = 'done';
    })
    .catch((e) => {
      current.state = 'failed: ' + String(e.message || e);
    });
}

// What the encoders made, for the app to write: { video: { description,
// chunks: [{ data, key }] }, sound: { codec, description, channels,
// chunks: [{ data, duration }] } | null }. Hands it over once.
export function bc_export_take() {
  if (!job || job.state !== 'done') return null;
  const taken = {
    video: { description: job.videoDescription, chunks: job.videoChunks },
    sound: job.sound
      ? {
          codec: job.soundCodec,
          description: job.soundDescription,
          channels: job.channels,
          chunks: job.soundChunks,
        }
      : null,
  };
  close(job);
  job = null;
  return taken;
}

function close(current) {
  for (const encoder of [current.video, current.sound]) {
    try {
      if (encoder && encoder.state !== 'closed') encoder.close();
    } catch (_) {
      // Already gone.
    }
  }
}

export function bc_export_cancel() {
  if (!job) return;
  job.state = 'cancelled';
  close(job);
  job = null;
}

// Call `callback` about fifty times a second while this tab is hidden. A
// hidden tab draws nothing, so the app's own frames stop, and its timers
// are slowed to one a second or less; a worker's are not, and its messages
// arrive on time — so an export goes on when the person looks at another tab.
let hiddenTicker = null;
export function bc_while_hidden(callback) {
  if (hiddenTicker) return;
  try {
    const source = 'setInterval(() => postMessage(0), 20);';
    hiddenTicker = new Worker(URL.createObjectURL(new Blob([source], { type: 'text/javascript' })));
  } catch (_) {
    return;
  }
  hiddenTicker.onmessage = () => {
    if (document.hidden) callback();
  };
}

// Hand the person a file, as a download.
export function bc_download(bytes, name, type) {
  const url = URL.createObjectURL(new Blob([bytes], { type }));
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

#!/usr/bin/env node
// Downloads the clips listed in samples/library.json into samples/library/.
//
// Every entry must be marked Creative Commons on YouTube; the script checks the
// license at download time and refuses anything else. Each clip is cut to its
// [start, end) window, re-encoded to h264/aac at 720p (or mp3 for audioOnly),
// and gets a poster frame for the library picker. Existing files are kept, so
// re-running only fetches what's missing. Pass --force to re-download all.
//
// Requires yt-dlp and ffmpeg on PATH.
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const outDir = join(here, 'library');
const force = process.argv.includes('--force');
const entries = JSON.parse(readFileSync(join(here, 'library.json'), 'utf8'));

mkdirSync(outDir, { recursive: true });

const run = (cmd, args) => execFileSync(cmd, args, { stdio: ['ignore', 'pipe', 'inherit'] });

let failed = 0;
for (const entry of entries) {
  if (!entry.youtubeId) continue; // local clips, e.g. the synthetic demo
  const ext = entry.audioOnly ? 'mp3' : 'mp4';
  const target = join(outDir, `${entry.slug}.${ext}`);
  const poster = join(outDir, `${entry.slug}.jpg`);
  if (!force && existsSync(target) && (entry.audioOnly || existsSync(poster))) {
    console.log(`✓ ${entry.slug} (cached)`);
    continue;
  }

  const url = `https://www.youtube.com/watch?v=${entry.youtubeId}`;
  try {
    const license = run('yt-dlp', ['--no-update', '--skip-download', '--print', '%(license)s', url])
      .toString()
      .trim();
    if (!license.startsWith('Creative Commons')) {
      throw new Error(`not Creative Commons on YouTube (license: ${license || 'standard'})`);
    }

    const raw = join(outDir, `${entry.slug}.raw`);
    rmSync(`${raw}.mp4`, { force: true });
    console.log(`↓ ${entry.slug}: ${entry.start}–${entry.end} s of ${url}`);
    run('yt-dlp', [
      '--no-update',
      '--quiet',
      '--no-warnings',
      '-f',
      'bv*[height<=720]+ba/b[height<=720]/b',
      '--download-sections',
      `*${entry.start}-${entry.end}`,
      '--merge-output-format',
      'mp4',
      '-o',
      `${raw}.%(ext)s`,
      url,
    ]);

    const encode = entry.audioOnly
      ? ['-vn', '-c:a', 'libmp3lame', '-b:a', '128k']
      : [
          '-vf',
          'scale=-2:min(720\\,ih)',
          '-c:v',
          'libx264',
          '-crf',
          '23',
          '-preset',
          'veryfast',
          '-pix_fmt',
          'yuv420p',
          '-c:a',
          'aac',
          '-b:a',
          '128k',
          '-movflags',
          '+faststart',
        ];
    run('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-y', '-i', `${raw}.mp4`, ...encode, target]);
    if (!entry.audioOnly) {
      run('ffmpeg', [
        '-hide_banner',
        '-loglevel',
        'error',
        '-y',
        '-ss',
        '3',
        '-i',
        target,
        '-frames:v',
        '1',
        '-vf',
        'scale=480:-2',
        '-q:v',
        '4',
        poster,
      ]);
    }
    rmSync(`${raw}.mp4`, { force: true });
    console.log(`✓ ${entry.slug}`);
  } catch (err) {
    failed += 1;
    console.error(`✗ ${entry.slug}: ${err instanceof Error ? err.message : err}`);
  }
}

if (failed > 0) {
  console.error(`${failed} clip(s) failed`);
  process.exit(1);
}

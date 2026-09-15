# samples/

Drop a short test clip here as `sample.mp4` (a 20-second talking-head video works well).
The ffmpeg integration test in `server/src/export.test.ts` looks for it and skips itself
if it is missing. Media files in this directory are ignored by git.

To make one from any video:

```sh
ffmpeg -ss 5 -t 20 -i input.mov -vf scale=-2:480 -c:v libx264 -crf 23 -c:a aac samples/sample.mp4
```

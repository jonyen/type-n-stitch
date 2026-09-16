# samples/

Starter media for trying the editor, plus the clip the engine's render test uses. Media files
in this directory are ignored by git; only the scripts and the manifest are committed.

```sh
npm run library    # make sample.mp4 and download every clip in library.json
```

## sample.mp4

A 20-second synthetic clip made by `make-sample.sh` from macOS `say`, with planted fillers
("um", "uh") and long pauses, over a waveform picture. `engine/tests/render.rs` renders cuts
and an overdub against it and expects exactly 20 seconds; it skips itself if the file is
missing. Pass a voice name to change the speaker: `samples/make-sample.sh Reed`.

## The library

`library.json` lists the clips shown under **Or start from a sample** in the app.
`fetch-library.mjs` downloads each YouTube entry's `start`–`end` window into `library/` with
yt-dlp, re-encodes it to 720p h264/aac (or mp3 for `audioOnly`), and saves a poster frame.
Re-running only fetches what's missing; `--force` re-downloads everything. If YouTube starts
answering 403, update yt-dlp (`brew upgrade yt-dlp`).

The server lists the manifest at `GET /api/library`, and at startup imports and transcribes
every downloaded clip in the background, so opening one is instant after the first run.

Every YouTube clip must be published under a Creative Commons license. The fetch script reads
the license from YouTube and refuses anything else. Don't add re-uploads of other people's
material, even if the uploader marked them CC.

| Clip                     | Source                                                                                                                   | Author                 | License |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------ | ---------------------- | ------- |
| `vlog-wednesday`         | [I just wanted to talk to the camera/a Wednesday vlog](https://www.youtube.com/watch?v=VOA_H5aCpG4)                      | Lucy Sims              | CC BY   |
| `vlog-camera-confidence` | [GETTING CAMERA CONFIDENCE (HOW TO VLOG)](https://www.youtube.com/watch?v=LiXw0vovqCc)                                   | Naveed Anwer           | CC BY   |
| `talk-educational-video` | [Derek Muller: The key to effective educational science videos](https://www.youtube.com/watch?v=RQaW2bFieo8)            | TED (TEDTalentSearch)  | CC BY   |
| `tedx-creativity`        | [Creativity as a Life Skill: Gerard Puccio at TEDxGramercy](https://www.youtube.com/watch?v=ltPAsp71rmI)                 | TEDx Talks             | CC BY   |
| `lecture-command-line`   | [Lecture 5: Command-line Environment (2020)](https://www.youtube.com/watch?v=e8BO_dYxk5c)                                | Missing Semester (MIT) | CC BY   |
| `lecture-yale-audio`     | [1. Introduction](https://www.youtube.com/watch?v=5_yOVARO2Oc)                                                           | YaleCourses            | CC BY   |
| `conversation-interview` | [How to Speak in an Interview \| Real English Conversation](https://www.youtube.com/watch?v=Qq9aYJxALAU)                 | Daily English Studio   | CC BY   |

The licenses are as YouTube reported them on 2026-09-16.

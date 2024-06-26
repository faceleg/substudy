# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.9] - 2024-04-11

### Fixed

- Build only substudy and dependencies, to avoid having builds fail due to `substudy-ui`.


## [0.6.8] - 2024-04-11

### Fixed

- dict_importer: Remove build dependency on `openssl`.

## [0.6.7] - 2024-04-11

### Fixed

- Fixed formatting issue which broken binary releases.
- Improved documentation for `transcribe --expected-text`.

## [0.6.6] - 2024-04-11

### Added

- Improved help messages for `substudy transcribe`.
- `transcribe` now supports `--expected-text`, which should be used when you believe that you have a more-or-less complete and accurate transcription already, which you wish to synchronize. This is especially useful for songs, where lyrics are readily available but the error rate may be higher. This mode also preserves line breaks.
- The `transcribe --example-text` option is now `--related-text`, in order to make the meaning clearer. The old option is still supported.
- `transcribe` now supports an optional `--lang` argument. This is most useful when the language is known, but the available "related text" is non-existant or extremely short. Normally, the language is detected automatically.

## [0.6.5] - 2024-03-30

### Added

- Run more transcription and translation requests in parallel. This greatly reduces the time needed to work with large media files.
- Cache AI API requests. Calling an AI model is slow and costs money. Making the same calls over and over again is a waste of time and money,
especially when we successfully process 99.5% of a large media file. So now we cache recent successful requests. So if you need to re-run a incomplete translation, it should be much faster and cheaper. (Cache files are stored wherever your OS thinks they should be stored. On Linux, this is `~/.cache/substudy`.)

## [0.6.4] - 2024-03-24

### Added

- We now support `substudy transcribe` with a `--format=whisper-srt` option. This outputs an SRT file generated directly by Whisper. This seems to have more artifacts than the default `--format=srt`, but it probably works a lot better for languages like Japanese that don't use spaces between words.

## [0.6.3] - 2024-03-20

### Fixed

- Linux: Do not use OpenSSL. It prevents us from building static binaries, and generally makes everything else harder.

## [0.6.2] - 2024-03-20

### Added

- Add `substudy export anki` for use with AnkiConnect. See the README.md file for an example.

### Fixed

- Correctly transcribe audio files that do not tag language information.

## [0.6.1] - 2024-03-17

### Added

- Audio-only files should now export a single album cover (if available) in place of video frames.

## [0.6.0] - 2024-03-16

### Added

- We now support Whisper-1 transcription of audio!
- Segmentation support using WebRTC voice audio detection, for finding good split points for speech recognition.
- Pretty progress bars! Now with more emoji!

### Fixed

- Translate slightly fewer lines at a time and tweak our prompt, to discourage returning the wrong number of lines.

### Removed

- `substudy` is no longer available as a library. Only the CLI is supported for the time being. We may make a library version available in the future, once the APIs have stabilized more.

## [0.5.2] - 2024-03-12

### Added

- Add automatic subtitle translation using `substudy translate foreign_subs.srt --native-lang=en > native_subs.srt`. This requires setting an `OPENAI_API_KEY` environment variable. (Or [a `.env` file](https://crates.io/crates/dotenv) containing that variable.) This is still experimental, but it worked well for Spanish-to-English translations in my test.

### Fixed

- Much prettier progress bars.

## [0.5.1] - 2024-03-10

### Changed

- Switched to the Apache 2.0 license for new code.

### Added

- Import Whisper JSON output using `substudy import whisper-json whisper.json > output.srt`. To create `whisper.json`, see [the `whisper.py` script](https://github.com/emk/subtitles-rs/blob/a7f9f03bdf45ea22550b9abe311bb473dd449cc3/python-experiments/whisper.py), which should work for audio files under 25 MB in major languages. We hope to integrate this into `substudy` in the future.

## [0.5.0] - 2024-03-07

### Fixed

- Updated all dependencies from their ancient 2017 versions to their latest versions.
- Ported code to Rust 2021 edition.
- Switched error-handling from the deprecated `failure` to `anyhow`, and removed dependency on `common_failures`.
- Moved binary builds to GitHub.

## [0.4.5] - 2017-12-08

### Added

- We now have official binaries for Linux, Mac and Windows.
- We now have a progress bar for media exports!

### Fixed

- The `uchardet` and `cld2` dependencies have been replaced with pure Rust dependencies. This makes it easier to support many platforms and to build from source.
- Argument parsing has been totally overhauled, so help messages should be better.
- Error formatting has been standardized and improved, so it's easier to figure out why something went wrong.
- We now support *.srt files generated by Aeneas, which is excellent for syncing audiobooks with text. These broke before because Aeneas occasionally generates 0-second subtitles, which we rejected as invalid.


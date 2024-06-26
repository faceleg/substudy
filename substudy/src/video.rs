//! Tools for working with video files.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    future::Future,
    path::{Path, PathBuf},
    process::Stdio,
    result,
    str::{from_utf8, FromStr},
};

use anyhow::{anyhow, Context as _};
use cast;
use log::debug;
use num::rational::Ratio;
use regex::Regex;
use serde::{de, Deserialize, Deserializer};
use serde_json;
use tokio::{
    io::{AsyncRead, BufReader},
    process::Command,
};

use crate::{
    errors::RunCommandError,
    lang::Lang,
    time::Period,
    ui::{ProgressConfig, Ui},
    Result,
};

/// The identifier of a data stream within a media container
/// format. This is used to refer to individual audio or video
/// streams within a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamId(usize);

/// Information about an MP3 track (optional).
#[derive(Clone, Debug, Default)]
#[allow(missing_docs)]
pub struct Id3Metadata {
    pub genre: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<(usize, usize)>,
    pub track_name: Option<String>,
    pub lyrics: Option<String>,
}

impl Id3Metadata {
    fn add_args(&self, cmd: &mut Command) {
        if let Some(ref genre) = self.genre {
            cmd.arg("-metadata").arg(format!("genre={}", genre));
        }
        if let Some(ref artist) = self.artist {
            cmd.arg("-metadata").arg(format!("artist={}", artist));
        }
        if let Some(ref album) = self.album {
            cmd.arg("-metadata").arg(format!("album={}", album));
        }
        if let Some((track, total)) = self.track_number {
            cmd.arg("-metadata")
                .arg(format!("track={}/{}", track, total));
        }
        if let Some(ref track_name) = self.track_name {
            cmd.arg("-metadata").arg(format!("title={}", track_name));
        }
        if let Some(ref lyrics) = self.lyrics {
            cmd.arg("-metadata").arg(format!("lyrics={}", lyrics));
        }
    }
}

/// A picture. This is basically the same as [`audiotags::types::Picture`], except
/// that it's `'static`.
pub struct Picture {
    /// The MIME type of the picture.
    pub mime_type: String,
    /// The picture data.
    pub data: Vec<u8>,
}

/// Individual streams inside a video are labelled with a codec type.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum CodecType {
    Audio,
    Video,
    Subtitle,
    Other(String),
}

impl<'de> Deserialize<'de> for CodecType {
    fn deserialize<D: Deserializer<'de>>(d: D) -> result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        match &s[..] {
            "audio" => Ok(CodecType::Audio),
            "video" => Ok(CodecType::Video),
            "subtitle" => Ok(CodecType::Subtitle),
            s => Ok(CodecType::Other(s.to_owned())),
        }
    }
}

/// A wrapper around `Ratio` with custom serialization support.
#[derive(Debug)]
pub struct Fraction(Ratio<u32>);

impl Fraction {
    fn deserialize_parts<'de, D>(d: D) -> result::Result<(u32, u32), D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(d)?;
        let re = Regex::new(r"^(\d+)/(\d+)$").unwrap();
        let cap = re.captures(&s).ok_or_else(|| {
            <D::Error as de::Error>::custom(format!("Expected fraction: {}", &s))
        })?;
        Ok((
            FromStr::from_str(cap.get(1).unwrap().as_str()).unwrap(),
            FromStr::from_str(cap.get(2).unwrap().as_str()).unwrap(),
        ))
    }
}

impl<'de> Deserialize<'de> for Fraction {
    fn deserialize<D: Deserializer<'de>>(d: D) -> result::Result<Self, D::Error> {
        let (num, denom) = Fraction::deserialize_parts(d)?;
        if denom == 0 {
            Err(<D::Error as de::Error>::custom(
                "Found fraction with a denominator of 0",
            ))
        } else {
            Ok(Fraction(Ratio::new(num, denom)))
        }
    }
}

/// An individual content stream within a video.
#[derive(Clone, Debug, Deserialize)]
#[allow(missing_docs)]
pub struct Stream {
    pub index: usize,
    pub codec_type: CodecType,
    pub tags: Option<BTreeMap<String, String>>,
    pub disposition: Option<BTreeMap<String, u32>>,
}

impl Stream {
    /// Return the language associated with this stream, if we can figure
    /// it out.
    pub fn language(&self) -> Option<Lang> {
        self.tags
            .as_ref()
            .and_then(|tags| tags.get("language"))
            .and_then(|lang| Lang::iso639(lang).ok())
    }

    /// Does this stream appear to be an attached picture? If so, this is
    /// probably album cover art attached to a music file, and we'll need to
    /// handle it specially.
    pub fn is_attached_pic(&self) -> bool {
        self.disposition
            .as_ref()
            .and_then(|d| d.get("attached_pic"))
            .map(|&v| v == 1)
            .unwrap_or(false)
    }
}

#[test]
fn test_stream_decode() {
    let json = "
{
  \"index\" : 2,
  \"codec_name\" : \"aac\",
  \"codec_long_name\" : \"AAC (Advanced Audio Coding)\",
  \"codec_type\" : \"audio\",
  \"codec_time_base\" : \"1/48000\",
  \"codec_tag_string\" : \"[0][0][0][0]\",
  \"codec_tag\" : \"0x0000\",
  \"sample_rate\" : \"48000.000000\",
  \"channels\" : 2,
  \"bits_per_sample\" : 0,
  \"avg_frame_rate\" : \"0/0\",
  \"time_base\" : \"1/1000\",
  \"start_time\" : \"0.000000\",
  \"duration\" : \"N/A\",
  \"tags\" : {
    \"language\" : \"eng\"
  }
}
";
    let stream: Stream = serde_json::from_str(json).unwrap();
    assert_eq!(CodecType::Audio, stream.codec_type);
    assert_eq!(Some(Lang::iso639("en").unwrap()), stream.language())
}

/// What kind of image source does this file contain?
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageSourceType {
    /// A true video stream, which presumably changes over time.
    Video,
    /// An attached picture, which is probably album art.
    AttachedPic,
}

/// What kind of data do we want to extract, and from what position in the
/// video clip?
#[derive(Clone)]
pub enum ExtractionSpec {
    /// Extract an image at the specified time.
    Image {
        /// Time from which to extract an image. This will only work with
        /// genuine video streams, not attached pictures.
        time: f32,
    },
    /// Extract an audio clip covering the specified stream and period.
    Audio {
        /// The stream to extract from, or `None` for the default.
        stream: Option<StreamId>,
        /// The period to extract.
        period: Period,
        /// Metadata to add to the extracted file.
        metadata: Id3Metadata,
    },
}

impl ExtractionSpec {
    /// The earliest time at which we might need to extract data.
    pub fn earliest_time(&self) -> f32 {
        match self {
            &ExtractionSpec::Image { time } => time,
            &ExtractionSpec::Audio { period, .. } => period.begin(),
        }
    }

    /// Can we combine this extraction with others in a giant batch
    /// request?
    fn can_be_batched(&self) -> bool {
        match self {
            // Batch processing of images requires decoding the whole
            // video, but we can do a "fast seek" and extract one image
            // extremely quickly.
            &ExtractionSpec::Image { .. } => false,
            _ => true,
        }
    }

    /// Figure out what ffmpeg args we would need to extract the requested
    /// data.  Assume that the "fast seek" feature has been used to start
    /// decoding at `time_base`.
    fn add_args(&self, cmd: &mut Command, time_base: f32) {
        match self {
            ExtractionSpec::Image { time } => {
                let scale_filter =
                    format!("scale=iw*min(1\\,min({}/iw\\,{}/ih)):-1", 1024, 768);
                cmd.arg("-ss")
                    .arg(format!("{}", time - time_base))
                    .arg("-vframes")
                    .arg("1")
                    .arg("-filter_complex")
                    .arg(&scale_filter)
                    .arg("-f")
                    .arg("image2");
            }
            ExtractionSpec::Audio {
                stream,
                period,
                metadata,
            } => {
                if let Some(sid) = stream {
                    cmd.arg("-map").arg(format!("0:{}", sid.0));
                }
                metadata.add_args(cmd);
                cmd.arg("-ss")
                    .arg(format!("{}", period.begin() - time_base))
                    .arg("-t")
                    .arg(format!("{}", period.duration()));
            }
        }
    }
}

/// Information about what kind of data we want to extract.
#[derive(Clone)]
pub struct Extraction {
    /// The path to extract to.
    pub path: PathBuf,
    /// What kind of data to extract.
    pub spec: ExtractionSpec,
}

impl Extraction {
    /// Add the necessary args to `cmd` to perform this extraction.
    fn add_args(&self, cmd: &mut Command, time_base: f32) {
        self.spec.add_args(cmd, time_base);
        cmd.arg(self.path.clone());
    }
}

/// Metadata associated with a video.
#[derive(Clone, Debug, Deserialize)]
struct Metadata {
    streams: Vec<Stream>,
}

/// Represents a video file on disk.
#[derive(Clone, Debug)]
pub struct Video {
    path: PathBuf,
    metadata: Metadata,
}

impl Video {
    /// Create a new video file, given a path.
    pub async fn new(path: &Path) -> Result<Video> {
        // Ensure we have an actual file before doing anything else.
        if !path.is_file() {
            return Err(anyhow!("No such file {:?}", path.display()));
        }

        // Run our probe command.
        let mkerr = || RunCommandError::new("ffprobe");
        let cmd = Command::new("ffprobe")
            .arg("-v")
            .arg("quiet")
            .arg("-show_streams")
            .arg("-of")
            .arg("json")
            .arg(path)
            .output()
            .await;
        let output = cmd.with_context(mkerr)?;
        let stdout = from_utf8(&output.stdout).with_context(mkerr)?;
        debug!("Video metadata: {}", stdout);
        let metadata = serde_json::from_str(stdout).with_context(mkerr)?;

        Ok(Video {
            path: path.to_owned(),
            metadata: metadata,
        })
    }

    /// Get just the file name of this video file.
    pub fn file_name(&self) -> &OsStr {
        self.path.file_name().unwrap()
    }

    /// Get just the file stem of this video file, stripped of any
    /// extensions.
    pub fn file_stem(&self) -> &OsStr {
        self.path.file_stem().unwrap()
    }

    /// List all the data streams in a video file.
    pub fn streams(&self) -> &[Stream] {
        &self.metadata.streams
    }

    /// Our primary video stream, or the closest equivalent.
    pub fn primary_video_stream(&self) -> Option<&Stream> {
        self.streams()
            .iter()
            .find(|s| s.codec_type == CodecType::Video)
    }

    /// What type of image source does this file contain?
    pub fn image_source_type(&self) -> Option<ImageSourceType> {
        let primary = self.primary_video_stream();
        match primary {
            Some(s) if s.is_attached_pic() => Some(ImageSourceType::AttachedPic),
            Some(_) => Some(ImageSourceType::Video),
            None => None,
        }
    }

    /// Choose the best audio for the specified language.
    pub fn audio_track_for(&self, lang: Lang) -> Option<StreamId> {
        self.streams()
            .iter()
            .position(|s| {
                s.codec_type == CodecType::Audio && s.language() == Some(lang)
            })
            .map(StreamId)
    }

    /// Create an extraction command using the specified `time_base`.  This
    /// allows us to start extractions at any arbitrary point in the video
    /// rapidly.
    fn extract_command(&self, time_base: f32) -> Command {
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-ss").arg(format!("{}", time_base));
        cmd.arg("-i").arg(&self.path);
        cmd
    }

    /// Perform a single extraction.
    async fn extract_one(&self, extraction: &Extraction) -> Result<()> {
        let time_base = extraction.spec.earliest_time();
        let mut cmd = self.extract_command(time_base);
        extraction.add_args(&mut cmd, time_base);
        cmd.output()
            .await
            .with_context(|| RunCommandError::new("ffmpg"))?;
        Ok(())
    }

    /// Perform a batch extraction.  We assume that the extractions are
    /// sorted in temporal order.
    async fn extract_batch(&self, extractions: &[&Extraction]) -> Result<()> {
        // Bail early if we have nothing to extract
        if extractions.is_empty() {
            return Ok(());
        }
        let time_base = extractions[0].spec.earliest_time();

        // Build and run our batch extraction command.
        let mut cmd = self.extract_command(time_base);
        for e in extractions {
            assert!(e.spec.can_be_batched());
            e.add_args(&mut cmd, time_base);
        }
        cmd.output()
            .await
            .with_context(|| RunCommandError::new("ffmpg"))?;
        Ok(())
    }

    /// Perform a list of extractions as efficiently as possible.  We use a
    /// batch interface to avoid making too many passes through the file.
    /// We assume that the extractions are sorted in temporal order.
    pub async fn extract(&self, ui: &Ui, extractions: &[Extraction]) -> Result<()> {
        let prog_conf = ProgressConfig {
            emoji: "✂️",
            msg: "Extracting media",
            done_msg: "Extracted media items",
        };
        let pb = ui.new_progress_bar(&prog_conf, cast::u64(extractions.len()));

        let mut batch: Vec<&Extraction> = vec![];
        for e in extractions {
            if e.spec.can_be_batched() {
                batch.push(e);
            } else {
                self.extract_one(e).await?;
                pb.inc(1);
            }
        }

        for chunk in batch.chunks(20) {
            self.extract_batch(chunk).await?;
            pb.inc(cast::u64(chunk.len()));
        }
        ui.finish(&prog_conf, pb);
        Ok(())
    }

    /// Get the attached picture from a "video" file. This typically happens
    /// when the video file is actually a music file with album art attached.
    /// Returns the file extension of the extracted image.
    pub fn attached_pic(&self) -> Result<Picture> {
        // Get our pic.
        let tag = audiotags::Tag::new()
            .read_from_path(&self.path)
            .with_context(|| {
                format!("could not read ID3 tags from {}", self.path.display())
            })?;
        let pic = tag.album_cover().ok_or_else(|| {
            anyhow!("no attached picture found in {}", self.path.display())
        })?;
        Ok(Picture {
            mime_type: String::from(pic.mime_type),
            data: pic.data.to_owned(),
        })
    }

    /// Open a stream from the video file as an async buffered reader.
    ///
    /// ```sh
    /// ffmpeg -i audio16000.mp3 -f s16le -ac 1 -ar 8000 -
    /// ```
    ///
    /// The stream will contain either 16-bit signed little-endian PCM or
    /// big-endian PCM, depending on the target architecture.
    pub async fn open_audio_stream(
        &self,
        stream: Option<StreamId>,
        rate: usize,
    ) -> Result<(BufReader<impl AsyncRead>, impl Future<Output = Result<()>>)> {
        let encoding = if cfg!(target_endian = "big") {
            "s16be"
        } else {
            "s16le"
        };

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-v").arg("quiet");
        cmd.arg("-i").arg(&self.path);
        if let Some(stream) = stream {
            cmd.arg("-map").arg(format!("0:{}", stream.0));
        }
        cmd.arg("-acodec").arg(format!("pcm_{}", encoding));
        cmd.arg("-f").arg(encoding);
        cmd.arg("-ac").arg("1");
        cmd.arg("-ar").arg(rate.to_string());
        cmd.arg("-");
        let mut child = cmd
            .stdout(Stdio::piped())
            .spawn()
            .with_context(|| RunCommandError::new("ffmpg"))?;
        let stdout = child.stdout.take().expect("should always have stdout");
        let join_handle = async move {
            let status = child.wait().await?;
            if !status.success() {
                Err(RunCommandError::new("ffmpg").into())
            } else {
                Ok(())
            }
        };
        Ok((BufReader::new(stdout), join_handle))
    }
}

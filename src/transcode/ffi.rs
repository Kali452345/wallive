//! Thin wrappers over the Media Foundation Source Reader / Sink Writer and
//! the D3D11 decoder-profile probe. All `unsafe` for `transcode/` lives here.
//!
//! Only runs in the short-lived import / test-clip process, never in the
//! resident wallpaper process.
#![allow(unsafe_code)]

use std::mem::ManuallyDrop;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0, D3D11_DECODER_PROFILE_H264_VLD_NOFGT,
    D3D11_DECODER_PROFILE_HEVC_VLD_MAIN, ID3D11Device, ID3D11VideoDevice,
};
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncMPVDefaultBPictureCount, CODECAPI_AVEncMPVGOPSize, ICodecAPI, IMFAttributes,
    IMFMediaType, IMFSample, IMFSinkWriter, IMFSourceReader, MF_MT_AVG_BITRATE,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
    MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SOURCE_READER_ALL_STREAMS,
    MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_VERSION, MFCreateAttributes, MFCreateMediaType,
    MFCreateMemoryBuffer, MFCreateSample, MFCreateSinkWriterFromURL, MFCreateSourceReaderFromURL,
    MFMediaType_Video, MFSTARTUP_FULL, MFShutdown, MFStartup, MFVideoFormat_H264,
    MFVideoFormat_NV12, MFVideoInterlace_Progressive, eAVEncH264VProfile_High,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_UI4};
use windows::core::{GUID, HSTRING, Interface, Result};

/// COM (MTA) + Media Foundation for the import process.
pub struct Platform(());

impl Platform {
    pub fn start() -> Result<Self> {
        // SAFETY: first COM call on this thread of the import process.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
        // SAFETY: MF startup with the bindings' SDK version.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }?;
        Ok(Self(()))
    }
}

impl Drop for Platform {
    fn drop(&mut self) {
        // SAFETY: balances MFStartup.
        unsafe {
            let _ = MFShutdown();
        }
    }
}

fn pack(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

fn attributes(count: u32) -> Result<IMFAttributes> {
    let mut attrs = None;
    // SAFETY: valid out-pointer.
    unsafe { MFCreateAttributes(&mut attrs, count) }?;
    attrs.ok_or_else(windows::core::Error::empty)
}

/// Video stream properties.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    /// Frames per second as numerator / denominator.
    pub fps: (u32, u32),
}

fn video_type(subtype: &windows::core::GUID, f: VideoFormat) -> Result<IMFMediaType> {
    // SAFETY: plain attribute setters on a fresh media type.
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, pack(f.width, f.height))?;
        t.SetUINT64(&MF_MT_FRAME_RATE, pack(f.fps.0, f.fps.1))?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        Ok(t)
    }
}

/// H.264 MP4 writer fed with NV12 samples. Uses a hardware encoder MFT when
/// one is available.
pub struct Encoder {
    writer: IMFSinkWriter,
    stream: u32,
}

impl Encoder {
    pub fn create(path: &std::path::Path, format: VideoFormat, bitrate: u32) -> Result<Self> {
        let attrs = attributes(1)?;
        // SAFETY: static GUID key; path HSTRING outlives the call.
        let writer = unsafe {
            attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
            MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None, &attrs)?
        };
        let out = video_type(&MFVideoFormat_H264, format)?;
        let input = video_type(&MFVideoFormat_NV12, format)?;
        // SAFETY: media types are fully populated; the writer copies them.
        let stream = unsafe {
            out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
            input.SetUINT32(&MF_MT_DEFAULT_STRIDE, format.width)?;
            let stream = writer.AddStream(&out)?;
            writer.SetInputMediaType(stream, &input, None)?;
            stream
        };
        // Best effort; the encoders tried so far accept both.
        // - No B-frames: playback decodes in low-latency mode, which assumes
        //   frames arrive in display order.
        // - A key frame every second: resuming after the decoder was released
        //   during a long pause restarts at most one second back (the
        //   default here was 128 frames).
        let gop = format.fps.0.div_ceil(format.fps.1.max(1)).max(1);
        for (api, value, what) in [
            (&CODECAPI_AVEncMPVDefaultBPictureCount, 0, "B-frames off"),
            (&CODECAPI_AVEncMPVGOPSize, gop, "key frame interval"),
        ] {
            if let Err(e) = set_codec_value(&writer, stream, api, value) {
                crate::log!("import: encoder rejected {what}: {e}");
            }
        }
        // SAFETY: stream and input type are set; starts the writer.
        unsafe { writer.BeginWriting()? };
        Ok(Self { writer, stream })
    }

    /// Writes one NV12 frame (`width * height * 3 / 2` bytes).
    pub fn write_nv12(&mut self, frame: &[u8], time_hns: i64, duration_hns: i64) -> Result<()> {
        let len = u32::try_from(frame.len()).map_err(|_| windows::core::Error::empty())?;
        // SAFETY: the buffer is locked for exactly `len` bytes, which we copy
        // into before unlocking; the sample takes a reference to the buffer.
        unsafe {
            let buffer = MFCreateMemoryBuffer(len)?;
            let mut data = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None)?;
            std::ptr::copy_nonoverlapping(frame.as_ptr(), data, frame.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(len)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            self.write_sample(&sample, time_hns, duration_hns)
        }
    }

    /// Writes a decoded NV12 sample from the source reader, re-timed.
    pub fn write_sample(
        &mut self,
        sample: &IMFSample,
        time_hns: i64,
        duration_hns: i64,
    ) -> Result<()> {
        // SAFETY: plain setters and a write on live interfaces.
        unsafe {
            sample.SetSampleTime(time_hns)?;
            sample.SetSampleDuration(duration_hns)?;
            self.writer.WriteSample(self.stream, sample)
        }
    }

    pub fn finish(self) -> Result<()> {
        // SAFETY: flushes and closes the file; no further writes follow.
        unsafe { self.writer.Finalize() }
    }
}

/// Sets one `ICodecAPI` value (VT_UI4) on the stream's encoder.
fn set_codec_value(writer: &IMFSinkWriter, stream: u32, api: &GUID, value: u32) -> Result<()> {
    let mut codec: Option<ICodecAPI> = None;
    let value = VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: VT_UI4,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 { ulVal: value },
            }),
        },
    };
    // SAFETY: `codec` is a valid out-pointer for an ICodecAPI of the
    // stream's encoder; a VT_UI4 VARIANT owns no memory.
    unsafe {
        writer.GetServiceForStream(
            stream,
            &GUID::zeroed(),
            &ICodecAPI::IID,
            &mut codec as *mut _ as *mut *mut std::ffi::c_void,
        )?;
        let codec = codec.ok_or_else(windows::core::Error::empty)?;
        codec.SetValue(api, &value)
    }
}

/// Decoding reader for the first video stream, with the MF video processor
/// doing resize, frame-rate conversion and conversion to NV12.
pub struct Decoder {
    reader: IMFSourceReader,
}

const FIRST_VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

impl Decoder {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let attrs = attributes(1)?;
        // SAFETY: static GUID key.
        unsafe { attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1) }?;
        Self::with_attributes(path, &attrs)
    }

    fn with_attributes(path: &std::path::Path, attrs: &IMFAttributes) -> Result<Self> {
        // SAFETY: path HSTRING outlives the call; attributes are populated.
        let reader =
            unsafe { MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), attrs)? };
        // SAFETY: stream selection on a live reader; audio is never decoded.
        unsafe {
            reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
            reader.SetStreamSelection(FIRST_VIDEO, true)?;
        }
        Ok(Self { reader })
    }

    /// The source's native video format.
    pub fn native_format(&self) -> Result<VideoFormat> {
        // SAFETY: plain getters on a live reader / media type.
        unsafe {
            let t = self.reader.GetNativeMediaType(FIRST_VIDEO, 0)?;
            let (width, height) = unpack(t.GetUINT64(&MF_MT_FRAME_SIZE)?);
            let fps = t
                .GetUINT64(&MF_MT_FRAME_RATE)
                .map(unpack)
                .unwrap_or((30, 1));
            Ok(VideoFormat { width, height, fps })
        }
    }

    /// Asks the reader to output NV12 in `format`.
    pub fn set_output(&self, format: VideoFormat) -> Result<()> {
        let t = video_type(&MFVideoFormat_NV12, format)?;
        // SAFETY: media type fully populated; reader copies it.
        unsafe { self.reader.SetCurrentMediaType(FIRST_VIDEO, None, &t) }
    }

    /// Next decoded frame and its timestamp, or `None` at end of stream.
    pub fn next(&self) -> Result<Option<(IMFSample, i64)>> {
        loop {
            let mut flags = 0u32;
            let mut time = 0i64;
            let mut sample: Option<IMFSample> = None;
            // SAFETY: all out-pointers are valid for this synchronous call.
            unsafe {
                self.reader.ReadSample(
                    FIRST_VIDEO,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut time),
                    Some(&mut sample),
                )?;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(None);
            }
            // Stream ticks and format changes arrive without a sample.
            if let Some(sample) = sample {
                return Ok(Some((sample, time)));
            }
        }
    }
}

/// Hardware decode support reported by the GPU driver (ADR-004).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodeSupport {
    pub h264: bool,
    pub hevc: bool,
    pub av1: bool,
}

pub fn probe_decoders(device: &ID3D11Device) -> Result<DecodeSupport> {
    let video: ID3D11VideoDevice = device.cast()?;
    let mut support = DecodeSupport::default();
    // SAFETY: indices are bounded by the reported count.
    unsafe {
        for i in 0..video.GetVideoDecoderProfileCount() {
            let Ok(p) = video.GetVideoDecoderProfile(i) else {
                continue;
            };
            support.h264 |= p == D3D11_DECODER_PROFILE_H264_VLD_NOFGT;
            support.hevc |= p == D3D11_DECODER_PROFILE_HEVC_VLD_MAIN;
            support.av1 |= p == D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0;
        }
    }
    Ok(support)
}

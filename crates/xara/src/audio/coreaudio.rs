//! Native 48 kHz PCM audio queues. All blocking work stays on the modem worker.
use super::{Audio, DeviceList, capture_only};
use std::ffi::{CStr, CString, c_char, c_void};
use std::io;

unsafe extern "C" {
    fn xara_audio_open(input: *const c_char, output: *const c_char, capture_only: i32, error: *mut i32) -> *mut c_void;
    fn xara_audio_close(audio: *mut c_void);
    fn xara_audio_read(audio: *mut c_void, samples: *mut i16, n: usize) -> i32;
    fn xara_audio_write(audio: *mut c_void, samples: *const i16, n: usize) -> i32;
    fn xara_audio_devices(
        visit: extern "C" fn(*mut c_void, *const c_char, *const c_char, i32),
        context: *mut c_void,
    ) -> i32;
}

fn error(code: i32) -> io::Error {
    io::Error::other(format!(
        "Core Audio error {code}: check device availability, sample format, and microphone permission"
    ))
}

pub struct CoreAudio {
    audio: *mut c_void,
    capture_only: bool,
}

impl CoreAudio {
    pub fn open(input: &str, output: &str, radio: bool, capture_only: bool) -> io::Result<Self> {
        check(output, radio)?;
        let input = CString::new(input).map_err(io::Error::other)?;
        let output = CString::new(output).map_err(io::Error::other)?;
        let mut code = 0;
        // SAFETY: strings and error out-pointer live through the synchronous open call.
        let audio = unsafe { xara_audio_open(input.as_ptr(), output.as_ptr(), i32::from(capture_only), &mut code) };
        if audio.is_null() { Err(error(code)) } else { Ok(Self { audio, capture_only }) }
    }
}

impl Audio for CoreAudio {
    fn read(&mut self, samples: &mut [i16]) -> io::Result<()> {
        // SAFETY: this instance exclusively owns audio; samples is writable for its full length.
        let code = unsafe { xara_audio_read(self.audio, samples.as_mut_ptr(), samples.len()) };
        if code == 0 { Ok(()) } else { Err(error(code)) }
    }

    fn write(&mut self, samples: &[i16]) -> io::Result<()> {
        if self.capture_only {
            return Err(capture_only());
        }
        // SAFETY: this instance exclusively owns audio; samples remains valid through the call.
        let code = unsafe { xara_audio_write(self.audio, samples.as_ptr(), samples.len()) };
        if code == 0 { Ok(()) } else { Err(error(code)) }
    }
}

impl Drop for CoreAudio {
    fn drop(&mut self) {
        // SAFETY: close synchronously stops callbacks before freeing the uniquely owned queues.
        unsafe { xara_audio_close(self.audio) };
    }
}

pub fn check(playback: &str, radio: bool) -> io::Result<()> {
    if radio && !playback.starts_with("coreaudio:") {
        return Err(io::Error::other("radio mode requires an explicitly selected Core Audio output device"));
    }
    if playback != "default" && !list()?.1.iter().any(|(name, _)| name == playback) {
        return Err(io::Error::other("selected Core Audio output device is unavailable"));
    }
    Ok(())
}

pub fn list() -> io::Result<(DeviceList, DeviceList)> {
    extern "C" fn visit(context: *mut c_void, name: *const c_char, label: *const c_char, input: i32) {
        // SAFETY: the C enumerator supplies valid strings for this callback; context points to lists below.
        unsafe {
            let lists = &mut *context.cast::<(DeviceList, DeviceList)>();
            let device = (
                CStr::from_ptr(name).to_string_lossy().into_owned(),
                CStr::from_ptr(label).to_string_lossy().into_owned(),
            );
            if input != 0 {
                lists.0.push(device);
            } else {
                lists.1.push(device);
            }
        }
    }
    let mut lists = (Vec::new(), Vec::new());
    // SAFETY: the enumerator invokes callbacks synchronously and retains no pointers.
    let code = unsafe { xara_audio_devices(visit, (&mut lists as *mut (DeviceList, DeviceList)).cast()) };
    if code == 0 { Ok(lists) } else { Err(error(code)) }
}

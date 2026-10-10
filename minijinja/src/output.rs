use std::ptr::addr_of_mut;
use std::{fmt, io};

use crate::error::{Error, ErrorKind};
use crate::utils::AutoEscape;
use crate::value::Value;

/// How should output be captured?
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[cfg_attr(feature = "unstable_machinery_serde", derive(serde::Serialize))]
pub enum CaptureMode {
    Capture,
    #[allow(unused)]
    Discard,
}

/// An abstraction over [`fmt::Write`](std::fmt::Write) for the rendering.
///
/// This is a utility type used in the engine which can be written into like one
/// can write into an [`std::fmt::Write`] value.  It's primarily used internally
/// in the engine but it's also passed to the custom formatter function.
pub struct Output<'a> {
    w: *mut (dyn fmt::Write + 'a),
    target: *mut (dyn fmt::Write + 'a),
    capture_stack: Vec<Option<String>>,
    /// One per frame --- the root, then one per capture --- while native
    /// captures are on; empty otherwise.
    frames: Vec<Frame>,
}

/// What a frame's output has been so far, for native captures.
#[derive(Default)]
struct Frame {
    written: Written,
    /// Inside an emission: writes are the value's own text.
    in_value: bool,
}

#[derive(Default)]
enum Written {
    #[default]
    Nothing,
    Sole(Value),
    Mixed,
}

impl<'a> Output<'a> {
    /// Creates a new output.
    pub(crate) fn new(w: &'a mut (dyn fmt::Write + 'a)) -> Self {
        Self {
            w,
            target: w,
            capture_stack: Vec::new(),
            frames: Vec::new(),
        }
    }

    /// Creates a null output that writes nowhere.
    pub(crate) fn null() -> Self {
        // The null writer also has a single entry on the discarding capture
        // stack.  In fact, `w` is more or less useless here as we always
        // shadow it.  This is done so that `is_discarding` returns true.
        Self {
            w: NullWriter::get_mut(),
            target: NullWriter::get_mut(),
            capture_stack: vec![None],
            frames: Vec::new(),
        }
    }

    /// Starts tracking frames for native captures, if it has not started.
    ///
    /// Called by the engine when the environment has native captures on. A
    /// frame is pushed for the root and for every capture already open, so
    /// the stacks stay aligned whenever tracking starts.
    pub(crate) fn track_native(&mut self) {
        while self.frames.len() <= self.capture_stack.len() {
            self.frames.push(Frame::default());
        }
    }

    /// Marks the start of one emitted value's text in the current frame.
    pub(crate) fn begin_value(&mut self, value: &Value) {
        if let Some(frame) = self.frames.last_mut() {
            frame.written = match frame.written {
                Written::Nothing => Written::Sole(value.clone()),
                _ => Written::Mixed,
            };
            frame.in_value = true;
        }
    }

    /// Marks the end of the value [`begin_value`](Self::begin_value) began.
    pub(crate) fn end_value(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            frame.in_value = false;
        }
    }

    /// Records a write that is not an emitted value's own text.
    #[inline(always)]
    fn note_write(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            if !frame.in_value {
                frame.written = Written::Mixed;
            }
        }
    }

    /// The root frame's sole value, if its whole output was one value that
    /// is not a string.
    pub(crate) fn sole_value(&mut self) -> Option<Value> {
        match self.frames.first_mut() {
            Some(frame) => native(std::mem::take(&mut frame.written)),
            None => None,
        }
    }

    /// Begins capturing into a string or discard.
    pub(crate) fn begin_capture(&mut self, mode: CaptureMode) {
        self.capture_stack.push(match mode {
            CaptureMode::Capture => Some(String::new()),
            CaptureMode::Discard => None,
        });
        if !self.frames.is_empty() {
            self.frames.push(Frame::default());
        }
        self.retarget();
    }

    /// Ends capturing and returns the captured string as value.
    ///
    /// With native captures tracked, a capture whose whole output was one
    /// value that is not a string returns that value instead.
    pub(crate) fn end_capture(&mut self, auto_escape: AutoEscape) -> Value {
        let sole = if self.frames.len() > 1 {
            self.frames.pop().and_then(|frame| native(frame.written))
        } else {
            None
        };
        let rv = if let Some(captured) = self.capture_stack.pop().unwrap() {
            if !matches!(auto_escape, AutoEscape::None) {
                Value::from_safe_string(captured)
            } else {
                Value::from(captured)
            }
        } else {
            Value::UNDEFINED
        };
        self.retarget();
        match sole {
            Some(value) if !rv.is_undefined() => value,
            _ => rv,
        }
    }

    fn retarget(&mut self) {
        self.target = match self.capture_stack.last_mut() {
            Some(Some(stream)) => stream,
            Some(None) => NullWriter::get_mut(),
            None => self.w,
        };
    }

    #[inline(always)]
    fn target(&mut self) -> &mut dyn fmt::Write {
        // SAFETY: this is safe because we carefully maintain the capture stack
        // to update self.target whenever it's modified
        unsafe { &mut *self.target }
    }

    /// Returns `true` if the output is discarding.
    #[inline(always)]
    #[allow(unused)]
    pub(crate) fn is_discarding(&self) -> bool {
        matches!(self.capture_stack.last(), Some(None))
    }

    /// Writes some data to the underlying buffer contained within this output.
    #[inline]
    pub fn write_str(&mut self, s: &str) -> fmt::Result {
        if !s.is_empty() {
            self.note_write();
        }
        self.target().write_str(s)
    }

    /// Writes some formatted information into this instance.
    #[inline]
    pub fn write_fmt(&mut self, a: fmt::Arguments<'_>) -> fmt::Result {
        self.note_write();
        self.target().write_fmt(a)
    }
}

impl fmt::Write for Output<'_> {
    #[inline]
    fn write_str(&mut self, s: &str) -> fmt::Result {
        Output::write_str(self, s)
    }

    #[inline]
    fn write_char(&mut self, c: char) -> fmt::Result {
        self.note_write();
        fmt::Write::write_char(self.target(), c)
    }

    #[inline]
    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> fmt::Result {
        Output::write_fmt(self, args)
    }
}

/// A frame's native result: its sole value, unless that is a string --- a
/// string is its own text, and answering the text keeps a safe string safe ---
/// or undefined, which a lenient engine renders as nothing.
///
/// A frame that wrote nothing at all is `none`, as Jinja2's native `concat`
/// answers `None` for an empty node list: an empty `{% set %}` block, an
/// empty macro or `caller()` body, a filter block over nothing and a
/// template whose output is empty are all `NoneType` in ansible-core.
fn native(written: Written) -> Option<Value> {
    match written {
        Written::Nothing => Some(Value::from(())),
        Written::Sole(value)
            if !matches!(
                value.kind(),
                crate::value::ValueKind::String | crate::value::ValueKind::Undefined
            ) =>
        {
            Some(value)
        }
        _ => None,
    }
}

pub struct NullWriter;

impl NullWriter {
    /// Returns a reference to the null writer.
    pub fn get_mut() -> &'static mut NullWriter {
        static mut NULL_WRITER: NullWriter = NullWriter;
        // SAFETY: this is safe as the null writer is a ZST
        unsafe { &mut *addr_of_mut!(NULL_WRITER) }
    }
}

impl fmt::Write for NullWriter {
    #[inline]
    fn write_str(&mut self, _s: &str) -> fmt::Result {
        Ok(())
    }

    #[inline]
    fn write_char(&mut self, _c: char) -> fmt::Result {
        Ok(())
    }
}

pub struct WriteWrapper<W> {
    pub w: W,
    pub err: Option<io::Error>,
}

impl<W> WriteWrapper<W> {
    /// Replaces the given error with the held error if available.
    pub fn take_err(&mut self, original: Error) -> Error {
        self.err
            .take()
            .map(|io_err| {
                Error::new(ErrorKind::WriteFailure, "I/O error during rendering")
                    .with_source(io_err)
            })
            .unwrap_or(original)
    }
}

impl<W: io::Write> fmt::Write for WriteWrapper<W> {
    #[inline]
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.w.write_all(s.as_bytes()).map_err(|e| {
            self.err = Some(e);
            fmt::Error
        })
    }

    #[inline]
    fn write_char(&mut self, c: char) -> fmt::Result {
        self.w
            .write_all(c.encode_utf8(&mut [0; 4]).as_bytes())
            .map_err(|e| {
                self.err = Some(e);
                fmt::Error
            })
    }
}

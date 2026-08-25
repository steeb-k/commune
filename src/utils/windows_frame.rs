//! The Win32 half of Commune's native window frame, on Windows.
//!
//! Commune runs GTK on Windows with server-side decorations (`GTK_CSD=0`, no
//! titlebar widget, see [`Window`](crate::window::Window)): GTK then gives the
//! `HWND` the normal frame styles (`WS_CAPTION | WS_THICKFRAME | WS_SYSMENU |
//! WS_MAXIMIZEBOX`) and keeps no shadow margin inside the window, so Windows
//! treats it like any other window -- Aero Snap, Win+Arrow, Snap Layouts, the
//! system menu, and the invisible resize borders that Windows lets overhang a
//! snap tile so the *visible* window fills it. What is left for this
//! subclass is the Chromium-style custom frame: `WM_NCCALCSIZE` keeps the
//! system's left/right/bottom resize borders but drops the caption (the
//! header bar is drawn by GTK in the client area), and `WM_NCHITTEST` says
//! which client pixels are the caption (drag = system move), the maximize
//! button (`HTMAXBUTTON` is what makes Snap Layouts appear) and the top
//! resize band. The toolkit supplies that geometry through a [`HitTester`].
//!
//! Ported from `ephemera-host-win32`'s `frame.rs` (see
//! `doc/windows-snapping-plan.md` for the measurements that justify this
//! approach), against the `windows` crate rather than hand-rolled bindings,
//! since Commune already depends on it. What in that reference lives on the
//! GTK side -- walking the picked widget to classify a point, and finding the
//! maximize button to light it up -- lives in `crate::window` instead, next
//! to the rest of the window's GTK code.

use std::cell::Cell;

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    Graphics::Gdi::ScreenToClient,
    UI::{
        Controls::WM_MOUSELEAVE,
        HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
        Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            GWL_STYLE, GetWindowLongPtrW, HTCAPTION, HTCLIENT, HTMAXBUTTON, HTTOP, IsZoomed,
            NCCALCSIZE_PARAMS, PostMessageW, SC_MAXIMIZE, SC_RESTORE, SM_CXPADDEDBORDER,
            SM_CYFRAME, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
            SetWindowLongPtrW, SetWindowPos, WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCHITTEST,
            WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE,
            WM_SYSCOMMAND, WS_MINIMIZEBOX,
        },
    },
};

/// What the toolkit says is under a client-area point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameHit {
    /// Ordinary content (the toolkit handles input there).
    Client,
    /// Title-bar area with nothing interactive: drag moves the window.
    Caption,
    /// The toolkit's maximize button.
    MaximizeButton,
}

/// `(x, y)` in physical pixels from the client area's top-left → what is there.
pub(crate) type HitTester = Box<dyn Fn(i32, i32) -> FrameHit>;

/// What the pointer is doing to the maximize button -- and, in `Away`, that
/// it is doing nothing to any of the window buttons.
///
/// Answering `HTMAXBUTTON` is what puts the Snap Layouts flyout up, but it
/// also hands that rectangle to Windows as *non-client*: the toolkit is sent
/// no motion there, so it neither lights the button up nor learns that the
/// pointer has left whichever button it was on before. This is the frame
/// telling it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MaximizeState {
    Away,
    Hover,
    Pressed,
}

/// Told whenever [`MaximizeState`] changes.
pub(crate) type MaximizeWatcher = Box<dyn Fn(MaximizeState)>;

const SUBCLASS_ID: usize = 0x4652; // "FR"

/// A `WM_NCHITTEST`/`WM_SYSCOMMAND`-family `u32` code as the `isize` these
/// Win32 APIs actually traffic in. Every code used here is small and
/// non-negative, so the round trip through `usize` never wraps.
fn code(value: u32) -> isize {
    (value as usize).cast_signed()
}

struct State {
    hit_test: HitTester,
    maximize: MaximizeWatcher,
    /// What was last reported, so a stream of `WM_NCMOUSEMOVE` over the same
    /// button is one call rather than one per pixel.
    reported: Cell<MaximizeState>,
}

impl State {
    fn report(&self, state: MaximizeState) {
        if self.reported.replace(state) != state {
            (self.maximize)(state);
        }
    }
}

/// Removes the subclass on drop.
#[derive(Debug)]
pub(crate) struct NativeFrame {
    hwnd: HWND,
    state: *mut State,
}

impl Drop for NativeFrame {
    fn drop(&mut self) {
        unsafe {
            let _ = RemoveWindowSubclass(self.hwnd, Some(subclass_proc), SUBCLASS_ID);
            drop(Box::from_raw(self.state));
        }
    }
}

/// Installs the custom-frame subclass on `hwnd` (before it is shown, so the
/// native caption never appears). The window must already carry the frame
/// styles -- GTK sets them for a decorated toplevel; `WS_MINIMIZEBOX`, which
/// GTK leaves alone, is added here.
pub(crate) fn install(
    hwnd: HWND,
    hit_test: HitTester,
    maximize: MaximizeWatcher,
) -> Option<NativeFrame> {
    unsafe {
        let state = Box::into_raw(Box::new(State {
            hit_test,
            maximize,
            reported: Cell::new(MaximizeState::Away),
        }));
        if SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, state as usize)
            .ok()
            .is_err()
        {
            drop(Box::from_raw(state));
            return None;
        }

        let style = GetWindowLongPtrW(hwnd, GWL_STYLE).cast_unsigned();
        let wanted = style | (WS_MINIMIZEBOX.0 as usize);
        if wanted != style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, wanted.cast_signed());
        }
        // Re-run WM_NCCALCSIZE now that the caption is ours to remove.
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );

        Some(NativeFrame { hwnd, state })
    }
}

/// Height of the system's top frame (the invisible resize border plus its
/// padding) at the window's DPI.
unsafe fn frame_height(hwnd: HWND) -> i32 {
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    ref_data: usize,
) -> LRESULT {
    unsafe {
        let state = &*(ref_data as *const State);
        match msg {
            // Let Windows lay out its frame, then take the caption back: the
            // client area starts at the window's top edge. Maximized, the
            // top border hangs off the monitor like the other three, so keep
            // that inset.
            WM_NCCALCSIZE if wparam.0 != 0 => {
                let params = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
                let top = params.rgrc[0].top;
                let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                params.rgrc[0].top = if IsZoomed(hwnd).as_bool() {
                    top + frame_height(hwnd)
                } else {
                    top
                };
                result
            }
            // The system's own answer covers the resize borders; client-area
            // points are the toolkit's to classify.
            WM_NCHITTEST => {
                let hit = DefSubclassProc(hwnd, msg, wparam, lparam);
                if hit.0 != code(HTCLIENT) {
                    return hit;
                }
                let mut point = POINT {
                    x: i32::from((lparam.0 & 0xFFFF) as i16),
                    y: i32::from(((lparam.0 >> 16) & 0xFFFF) as i16),
                };
                let _ = ScreenToClient(hwnd, &raw mut point);
                if !IsZoomed(hwnd).as_bool() && point.y < frame_height(hwnd) {
                    // The caption took the top border's place; give it back.
                    return LRESULT(code(HTTOP));
                }
                match (state.hit_test)(point.x, point.y) {
                    FrameHit::Client => LRESULT(code(HTCLIENT)),
                    FrameHit::Caption => LRESULT(code(HTCAPTION)),
                    FrameHit::MaximizeButton => LRESULT(code(HTMAXBUTTON)),
                }
            }
            // The pointer over the maximize button, which Windows now owns.
            // Asking for WM_NCMOUSELEAVE is what catches it going somewhere
            // that sends no message of its own -- back into the client area.
            WM_NCMOUSEMOVE => {
                if wparam.0 == HTMAXBUTTON as usize {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&raw mut track);
                    state.report(MaximizeState::Hover);
                } else {
                    state.report(MaximizeState::Away);
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            // The pointer moving from a window button onto the page crosses
            // into another window, and the toolkit is left holding a lit
            // button. Tracking is armed here rather than relied upon: the
            // toolkit's own is for its own purposes and may have lapsed.
            WM_MOUSEMOVE => {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&raw mut track);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            // The pointer leaving the maximize button's rectangle (caught
            // above by `WM_NCMOUSEMOVE`'s own tracking) or leaving the
            // client area onto a window button: either way, nothing is
            // hovered any more.
            WM_NCMOUSELEAVE | WM_MOUSELEAVE => {
                state.report(MaximizeState::Away);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            // Clicks on the maximize button arrive as non-client clicks now;
            // the toolkit never sees them, so act here.
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam.0 == HTMAXBUTTON as usize => {
                state.report(MaximizeState::Pressed);
                LRESULT(0)
            }
            WM_NCLBUTTONUP if wparam.0 == HTMAXBUTTON as usize => {
                state.report(MaximizeState::Hover);
                let command = if IsZoomed(hwnd).as_bool() {
                    SC_RESTORE
                } else {
                    SC_MAXIMIZE
                };
                let _ = PostMessageW(
                    Some(hwnd),
                    WM_SYSCOMMAND,
                    WPARAM(command as usize),
                    LPARAM(0),
                );
                LRESULT(0)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

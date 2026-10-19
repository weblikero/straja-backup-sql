//! Native notification icon owned by the SQL agent. No server or global shell settings.
use crate::i18n::{self, tr};
use crate::{model::Run, store::Store};
use anyhow::{Context, Result, bail};
use std::{
    cell::Cell,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::*, WindowsAndMessaging::*},
};

pub const WINDOW_CLASS: &str = "StrajaBackupSQL.Agent.Tray";
const ICON_ID: u32 = 1;
const CALLBACK: u32 = WM_APP + 42;
const OPEN: u32 = 101;
const ACTIVITY: u32 = 102;
const STOP: u32 = 103;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn tip(text: &str) -> [u16; 128] {
    let mut out = [0; 128];
    let mut used = 0;
    for c in text.chars().filter(|c| *c != '\0') {
        let mut units = [0; 2];
        let encoded = c.encode_utf16(&mut units);
        if used + encoded.len() >= out.len() {
            break;
        }
        out[used..used + encoded.len()].copy_from_slice(encoded);
        used += encoded.len();
    }
    out
}

pub fn status_text(runs: &[Run], stopping: bool) -> String {
    if stopping {
        return tr("StrajaBackupSQL · Stopping after current backup").into();
    }
    if let Some(run) = runs.iter().find(|r| r.status == "În lucru") {
        if let Some(p) = &run.progress {
            let percent = p
                .percent()
                .map(|n| format!(" · {n:.0}%"))
                .unwrap_or_default();
            return format!(
                "StrajaBackupSQL · {} · {}{percent}",
                i18n::label(&p.phase),
                p.database
            );
        }
        return i18n::format(
            "StrajaBackupSQL · Running: {0}",
            std::slice::from_ref(&run.plan_name),
        );
    }
    if runs
        .first()
        .is_some_and(|r| r.status == "Eroare" || r.status == "Întrerupt")
    {
        tr("StrajaBackupSQL · Agent running · Last backup needs attention").into()
    } else {
        tr("StrajaBackupSQL · Agent running").into()
    }
}

pub fn request_stop(root: &Path) -> Result<()> {
    Store::open(root)?.put("agent", "stop", &true)
}

pub fn open_window(activity: bool) -> Result<()> {
    let path = std::env::current_exe()?.with_file_name("StrajaBackupSQL.exe");
    let mut cmd = crate::engine::command(path);
    if activity {
        cmd.arg("--activity");
    }
    cmd.spawn()
        .context(tr("Could not open the StrajaBackupSQL window."))?;
    Ok(())
}

struct State {
    root: PathBuf,
    store: Store,
    last_tip: Cell<Option<[u16; 128]>>,
    last_probe: Cell<Instant>,
    icon: HICON,
    added: Cell<bool>,
    reported: Cell<Option<bool>>,
    taskbar_created: u32,
}
impl State {
    fn data(&self, hwnd: HWND) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: ICON_ID,
            ..Default::default()
        }
    }
    fn refresh(&self, hwnd: HWND) {
        i18n::reload_from(&self.store);
        let status = (|| -> Result<String> {
            Ok(status_text(
                &self.store.tray_runs()?,
                self.store.get("agent", "stop").unwrap_or(false),
            ))
        })()
        .unwrap_or_else(|_| tr("StrajaBackupSQL · Agent running · Check settings").into());
        let tooltip = tip(&status);
        if self.added.get()
            && self.last_tip.get() == Some(tooltip)
            && self.last_probe.get().elapsed() < Duration::from_secs(30)
        {
            return;
        }
        self.last_tip.set(Some(tooltip));
        self.last_probe.set(Instant::now());
        let mut data = self.data(hwnd);
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = CALLBACK;
        data.hIcon = self.icon;
        data.szTip = tooltip;
        // SAFETY: HWND and shared HICON live on this message-loop thread; data is initialized.
        unsafe {
            if self.added.get() {
                if Shell_NotifyIconW(NIM_MODIFY, &data) == 0 {
                    self.added.set(false);
                }
            } else if Shell_NotifyIconW(NIM_ADD, &data) != 0 {
                self.added.set(true);
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &data);
            }
        }
        if self.reported.get() != Some(self.added.get()) {
            let _ = self
                .store
                .put("agent", "tray_registered", &self.added.get());
            self.reported.set(Some(self.added.get()));
        }
    }
    fn action(&self, command: u32, hwnd: HWND) {
        let result = match command {
            OPEN => open_window(false),
            ACTIVITY => open_window(true),
            STOP => request_stop(&self.root),
            _ => Ok(()),
        };
        if let Err(error) = result {
            unsafe {
                MessageBoxW(
                    hwnd,
                    wide(&format!("{error:#}")).as_ptr(),
                    wide("StrajaBackupSQL").as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        self.refresh(hwnd);
    }
    fn menu(&self, hwnd: HWND) {
        i18n::reload_from(&self.store);
        // Immutable State/Cell is intentional: TrackPopupMenu runs a nested message loop.
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            AppendMenuW(
                menu,
                MF_STRING,
                OPEN as usize,
                wide(tr("Open StrajaBackupSQL")).as_ptr(),
            );
            AppendMenuW(
                menu,
                MF_STRING,
                ACTIVITY as usize,
                wide(tr("Backup activity")).as_ptr(),
            );
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
            let stopping = Store::open(&self.root)
                .and_then(|s| s.get::<bool>("agent", "stop"))
                .unwrap_or(false);
            AppendMenuW(
                menu,
                MF_STRING | if stopping { MF_GRAYED } else { 0 },
                STOP as usize,
                wide(if stopping {
                    tr("Stop requested…")
                } else {
                    tr("Stop agent after current backup")
                })
                .as_ptr(),
            );
            SetMenuDefaultItem(menu, OPEN, 0);
            let mut point = POINT::default();
            GetCursorPos(&mut point);
            SetForegroundWindow(hwnd);
            let choice = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                hwnd,
                null(),
            );
            DestroyMenu(menu);
            PostMessageW(hwnd, WM_NULL, 0, 0);
            if choice != 0 {
                self.action(choice as u32, hwnd);
            }
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // No Rust panic may cross the Win32 callback boundary.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        if message == WM_NCCREATE {
            let create = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
        if !state.is_null() {
            let state = &*state;
            if message == state.taskbar_created && message != 0 {
                state.added.set(false);
                state.refresh(hwnd);
                return 0;
            }
            match message {
                WM_TIMER => {
                    state.refresh(hwnd);
                    return 0;
                }
                CALLBACK => {
                    let event = (l as u32) & 0xffff;
                    if event == WM_CONTEXTMENU {
                        state.menu(hwnd);
                    } else if event == NIN_SELECT || event == NIN_SELECT | NINF_KEY {
                        state.action(OPEN, hwnd);
                    }
                    return 0;
                }
                WM_CLOSE => {
                    EndMenu();
                    DestroyWindow(hwnd);
                    return 0;
                }
                WM_DESTROY => {
                    KillTimer(hwnd, 1);
                    Shell_NotifyIconW(NIM_DELETE, &state.data(hwnd));
                    state.added.set(false);
                    if let Ok(store) = Store::open(&state.root) {
                        let _ = store.put("agent", "tray_registered", &false);
                    }
                    PostQuitMessage(0);
                    return 0;
                }
                WM_NCDESTROY => {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, w, l)
    }))
    .unwrap_or_else(|_| unsafe { DefWindowProcW(hwnd, message, w, l) })
}

pub struct Tray {
    hwnd: usize,
    thread: Option<thread::JoinHandle<()>>,
}
impl Tray {
    pub fn start(root: PathBuf) -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("StrajaBackupSQL.Tray".into())
            .spawn(move || {
                let result = unsafe { message_loop(root, &tx) };
                if let Err(error) = result {
                    let _ = tx.send(Err(format!("{error:#}")));
                }
            })?;
        match rx.recv().context(tr("The tray icon thread has closed."))? {
            Ok(hwnd) => Ok(Self {
                hwnd,
                thread: Some(thread),
            }),
            Err(error) => {
                let _ = thread.join();
                bail!("{error}");
            }
        }
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

unsafe fn message_loop(
    root: PathBuf,
    ready: &mpsc::SyncSender<Result<usize, String>>,
) -> Result<()> {
    // All HWND/HICON access stays on this thread; the owner only posts WM_CLOSE.
    unsafe {
        let instance = GetModuleHandleW(null());
        let class_name = wide(WINDOW_CLASS);
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            bail!("{}", tr("Cannot create the tray icon class."));
        }
        // MAKEINTRESOURCEW(1): the integer resource ID, not an address to dereference.
        let icon = LoadIconW(instance, std::ptr::without_provenance::<u16>(1));
        if icon.is_null() {
            UnregisterClassW(class_name.as_ptr(), instance);
            bail!(
                "{}",
                tr("The StrajaBackupSQL icon is missing from the agent.")
            );
        }
        let state = Box::new(State {
            store: Store::open(&root)?,
            last_tip: Cell::new(None),
            last_probe: Cell::new(Instant::now()),
            root,
            icon,
            added: Cell::new(false),
            reported: Cell::new(None),
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            wide("StrajaBackupSQL Agent").as_ptr(),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            (&*state as *const State).cast(),
        );
        if hwnd.is_null() {
            UnregisterClassW(class_name.as_ptr(), instance);
            bail!("{}", tr("Cannot create the hidden tray icon window."));
        }
        state.refresh(hwnd);
        SetTimer(hwnd, 1, 2000, None);
        if ready.send(Ok(hwnd as usize)).is_err() {
            DestroyWindow(hwnd);
        } else {
            let mut message = MSG::default();
            while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if IsWindow(hwnd) != 0 {
                DestroyWindow(hwnd);
            }
        }
        UnregisterClassW(class_name.as_ptr(), instance);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tooltip_is_terminated_and_does_not_split_surrogates() {
        let result = tip(&"🛡".repeat(100));
        let end = result.iter().position(|c| *c == 0).unwrap();
        assert!(end < 128);
        assert!(String::from_utf16(&result[..end]).is_ok());
        assert!(status_text(&[], false).contains("Agent running"));
        assert!(status_text(&[], true).contains("Stopping"));
    }
    #[test]
    fn tray_stop_and_window_requests_use_only_the_selected_profile() {
        let temp = tempfile::tempdir().unwrap();
        request_stop(temp.path()).unwrap();
        let store = Store::open(temp.path()).unwrap();
        assert!(store.get::<bool>("agent", "stop").unwrap());
        assert!(store.take_window_request().unwrap().is_none());
        store.request_window(true).unwrap();
        assert_eq!(store.take_window_request().unwrap(), Some(true));
        assert!(store.take_window_request().unwrap().is_none());
        store.request_window(false).unwrap();
        assert_eq!(store.take_window_request().unwrap(), Some(false));
    }
}

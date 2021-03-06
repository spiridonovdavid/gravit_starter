#![windows_subsystem = "windows"]
extern crate native_windows_gui as nwg;

use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::Duration;

use nwg::{HTextAlign, NativeUi, Notice, NoticeSender, NwgError};
use once_cell::sync::Lazy;

use crate::config::Config;
use crate::jre::get_jre;
use crate::launcher::{launcher_exist, run_launcher};
use crate::util::get_pointer_width;
use anyhow::Error;
use dirs::data_dir;
use std::path::{Path, PathBuf};

mod config;
mod jre;
mod launcher;
mod util;

pub static CONFIG: Lazy<Config> = Lazy::new(Config::default);
static BACKGROUND_DATA: &'static [u8] = include_bytes!("../background.bmp");

pub struct DownloadUi {
    inner: Rc<Download>,
    default_handler: RefCell<Option<nwg::EventHandler>>,
}

#[derive(Default)]
pub struct Download {
    window: nwg::Window,

    splash: nwg::Bitmap,

    layout: nwg::GridLayout,

    notice: nwg::Notice,

    label: nwg::Label,

    progress: nwg::ProgressBar,

    background: nwg::ImageFrame,

    recv: Option<Receiver<u64>>,
}

impl NativeUi<DownloadUi> for Download {
    fn build_ui(mut data: Self) -> Result<DownloadUi, NwgError> {
        use nwg::Event as E;
        let mut font = nwg::Font::default();

        let em = nwg::EmbedResource::load(None).unwrap();

        nwg::Font::builder()
            .size(16)
            .family("Arial")
            .weight(500)
            .build(&mut font);

        nwg::Window::builder()
            .flags(nwg::WindowFlags::WINDOW | nwg::WindowFlags::VISIBLE)
            .size((300, 115))
            .position((300, 300))
            .title(&CONFIG.title)
            .icon(em.icon_str("MAINICON", None).as_ref())
            .build(&mut data.window)?;

        nwg::Label::builder()
            .text("Download JRE")
            .font(Some(&font))
            .h_align(HTextAlign::Center)
            .parent(&data.window)
            .build(&mut data.label);

        nwg::Bitmap::builder()
            .source_bin(Some(BACKGROUND_DATA))
            .strict(true)
            .build(&mut data.splash)?;

        nwg::ProgressBar::builder()
            .state(nwg::ProgressBarState::Normal)
            .step(10)
            .range(0..3)
            .parent(&data.window)
            .build(&mut data.progress)?;

        nwg::ImageFrame::builder()
            .parent(&data.window)
            .bitmap(Some(&data.splash))
            .size((300, 115))
            .build(&mut data.background)?;

        nwg::Notice::builder()
            .parent(&data.window)
            .build(&mut data.notice)?;

        let (send, recv) = std::sync::mpsc::channel();

        data.recv = Some(recv);

        let ui = DownloadUi {
            inner: Rc::new(data),
            default_handler: Default::default(),
        };
        // Events
        let evt_ui = Rc::downgrade(&ui.inner);
        let handle_events = move |evt, _evt_data, handle| {
            if let Some(ui) = evt_ui.upgrade() {
                match evt {
                    E::OnInit => {
                        let notice = ui.notice.sender();
                        let send = send.clone();
                        thread::spawn(move || run_downloader(send, notice));
                    }
                    E::OnWindowClose => {
                        if &handle == &ui.window {
                            nwg::stop_thread_dispatch();
                        }
                    }
                    E::OnNotice => {
                        let state = ui.recv.as_ref().unwrap().recv().unwrap();
                        if state == 1 {
                            ui.label.set_text("Extract JRE")
                        }
                        if state == 2 {
                            ui.label.set_text("Download Launcher")
                        }
                        if state == 3 {
                            ui.label.set_text("Starting launcher")
                        }
                        if state == 10 {
                            nwg::stop_thread_dispatch();
                        } else if state == 11 {
                            nwg::modal_error_message(
                                &ui.window,
                                "Произошла ошибка",
                                "Произошла ошибка при запуске лаунчера.",
                            );
                            nwg::stop_thread_dispatch();
                        }
                        ui.progress.set_pos(state as u32);
                    }
                    _ => {}
                }
            }
        };
        *ui.default_handler.borrow_mut() = Some(nwg::full_bind_event_handler(
            &ui.window.handle,
            handle_events,
        ));

        nwg::GridLayout::builder()
            .parent(&ui.window)
            .spacing(1)
            .margin([30, 10, 30, 10])
            .spacing(1)
            .child(0, 3, &ui.label)
            .child_item(nwg::GridLayoutItem::new(&ui.progress, 0, 0, 1, 2))
            .build(&ui.layout)?;

        return Ok(ui);
    }
}

macro_rules! error {
    ($sender:expr, $notice:expr) => {
        $sender.send(11);
        $notice.notice();
        panic!()
    };
}

macro_rules! update {
    ($sender:expr, $notice:expr, $x:expr) => {
        $sender.send($x);
        $notice.notice();
    };
}

pub fn run_downloader(send: Sender<u64>, notice: NoticeSender) {
    let jre_path = match get_jre() {
        None => {
            if jre::download_jre().is_err() {
                error!(send, notice);
            } else {
                update!(send, notice, 1);
            }
            if jre::extract_jre().is_err() {
                error!(send, notice);
            } else {
                update!(send, notice, 2);
            }
            get_jre().unwrap()
        }
        Some(jre) => jre,
    };
    if !launcher::launcher_exist() {
        if launcher::download_launcher().is_err() {
            error!(send, notice);
        }
    }
    update!(send, notice, 3);
    if launcher::launcher_exist() {
        if run_launcher(jre_path.as_path()).is_err() {
            error!(send, notice);
        } else {
            update!(send, notice, 10);
        }
    } else {
        error!(send, notice);
    }
}

impl Drop for DownloadUi {
    fn drop(&mut self) {
        let handler = self.default_handler.borrow();
        if handler.is_some() {
            nwg::unbind_event_handler(handler.as_ref().unwrap());
        }
    }
}

impl Deref for DownloadUi {
    type Target = Download;

    fn deref(&self) -> &Download {
        &self.inner
    }
}

fn main() {
    let jre = get_jre();
    if launcher_exist() && jre.is_some() {
        run_launcher(jre.unwrap().as_path());
    } else {
        nwg::init().expect("Failed to init Native Windows GUI");
        nwg::Font::set_global_family("Segoe UI").expect("Failed to set default font");
        let _ui = Download::build_ui(Default::default()).expect("Failed to build UI");
        nwg::dispatch_thread_events();
    }
}

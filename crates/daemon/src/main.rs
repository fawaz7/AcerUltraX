//! `acerultrax-daemon`: runs Acer's DTS processing on the system's audio.

mod config;
mod control;
mod filter;
mod layout;
mod router;
mod volume;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use importer::Manifest;
use router::{Options, Router};

fn usage() -> ! {
    eprintln!("usage: acerultrax-daemon [--no-default] [--target <sink node name>]");
    std::process::exit(2);
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut opts = Options { no_default: false, target: None };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--no-default" => opts.no_default = true,
            "--target" => opts.target = Some(args.next().unwrap_or_else(|| usage())),
            _ => usage(),
        }
    }

    let data = importer::data_dir();
    let manifest = Manifest::load(&data).context("the DTS files are not set up yet (run `acerultrax-cli import`)")?;
    let lib = dts_eagle::Library::load(&Manifest::engine_path(&data), Some(&Manifest::dts_dir(&data)))?;
    let oem = Arc::new(dts_eagle::OemConfig::parse(&std::fs::read_to_string(manifest.oem_config_path(&data))?)?);

    // Block the stop signals before any thread starts, so every thread inherits the mask and only PipeWire's
    // signal handler (below) sees them; otherwise one could land on a worker thread and kill the process.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
    pipewire::init();
    let mainloop = pipewire::main_loop::MainLoopRc::new(None)?;
    let context = pipewire::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None).context("cannot connect to PipeWire")?;
    // If PipeWire goes away (restart, crash), exit with an error so the service manager starts us again.
    let lost = std::rc::Rc::new(std::cell::Cell::new(false));
    let (ml, l) = (mainloop.clone(), lost.clone());
    let _core_listener = core
        .add_listener_local()
        .error(move |id, _, res, msg| {
            if id == pipewire::core::PW_ID_CORE && res == -libc::EPIPE {
                log::error!("lost the connection to PipeWire: {msg}");
                l.set(true);
                ml.quit();
            }
        })
        .register();
    let router = Router::new(core, mainloop.loop_(), lib, oem, opts)?;

    let registry = router.borrow().registry();
    let (r1, r2) = (router.clone(), router.clone());
    let _reg = registry
        .add_listener_local()
        .global(move |g| Router::on_global(&r1, g))
        .global_remove(move |id| Router::on_global_remove(&r2, id))
        .register();

    let r = router.clone();
    let timer = mainloop.loop_().add_timer(move |_| r.borrow_mut().tick());
    timer.update_timer(Some(Duration::from_millis(100)), Some(Duration::from_millis(100))).into_result()?;

    let (tx, rx) = pipewire::channel::channel::<control::Request>();
    let r = router.clone();
    let _rx = rx.attach(mainloop.loop_(), move |(req, reply)| {
        let _ = reply.send(control::handle(&mut r.borrow_mut(), &req));
    });
    control::serve(tx).context("cannot open the control socket")?;

    let ml = mainloop.clone();
    let quit = move || ml.quit();
    let _sigint = mainloop.loop_().add_signal_local(pipewire::loop_::Signal::INT, quit.clone());
    let _sigterm = mainloop.loop_().add_signal_local(pipewire::loop_::Signal::TERM, quit);

    log::info!("running");
    mainloop.run();
    log::info!("stopping");
    router.borrow_mut().shutdown();
    // Let PipeWire receive the last changes before disconnecting.
    let ml = mainloop.clone();
    let done = mainloop.loop_().add_timer(move |_| ml.quit());
    done.update_timer(Some(Duration::from_millis(200)), None).into_result()?;
    mainloop.run();
    let _ = std::fs::remove_file(control::socket_path());
    if lost.get() {
        std::process::exit(1);
    }
    Ok(())
}

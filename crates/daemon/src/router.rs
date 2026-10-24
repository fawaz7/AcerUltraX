//! The PipeWire side, on the main loop:
//!
//! ```text
//! app stream ──▶ acerultrax_in_<serial> (hidden sink in the app's layout) ─┐
//! app stream ──▶ acerultrax_in_<serial> ...                                ├▶ acerultrax_output (filter) ─▶ device
//! new stream ──▶ acerultrax_sink ("AcerUltraX", the default output) ───────┘
//! ```
//!
//! Apps play into the visible sink (it is the default output); each new stream is moved to its own hidden sink
//! so it gets its own DTS stream engine, as on Windows. The filter's output is linked to the chosen device:
//! the one the user picked most recently that is present, else the one PipeWire ranks highest. Picking a
//! device in the desktop's sound settings makes AcerUltraX follow it. The visible sink's volume is mirrored to
//! the device, so it applies after DTS, as on Windows.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use pipewire::core::CoreRc;
use pipewire::link::{Link, LinkListener, LinkState};
use pipewire::metadata::{Metadata, MetadataListener};
use pipewire::node::{Node, NodeListener};
use pipewire::properties::properties;
use pipewire::registry::{GlobalObject, RegistryRc};
use pipewire::spa::param::ParamType;
use pipewire::spa::pod::Pod;
use pipewire::spa::utils::dict::DictRef;
use pipewire::types::ObjectType;

use crate::config::{Config, Kind, Profile, State};
use crate::filter::{Filter, MAX_GROUPS, Shared};
use crate::{layout, volume};

pub const SINK_NAME: &str = "acerultrax_sink";
pub const FILTER_NAME: &str = "acerultrax_output";
const IN_PREFIX: &str = "acerultrax_in_";
const DEFAULT_KEY: &str = "default.configured.audio.sink";
/// Stream id the chain uses for group 0 (streams not moved yet).
const SHARED_ID: u32 = 1;

pub struct Options {
    /// Leave the default output alone (for tests).
    pub no_default: bool,
    /// Always use this device (node name) instead of following the user's choice.
    pub target: Option<String>,
}

struct NodeInfo {
    name: String,
    description: String,
    class: String,
    priority: i32,
    bluetooth: bool,
    /// A USB audio device (headsets and dongles): may take a headphone model.
    usb: bool,
    hardware: bool,
    /// The PipeWire device it belongs to (its form factor tells headphones from speakers).
    device: Option<u32>,
}

struct PortInfo {
    node: u32,
    output: bool,
    name: String,
}

struct App {
    serial: u32,
    dont_move: bool,
    /// Its properties are known (from the node info; the registry announces only a few).
    info: bool,
    /// A native PipeWire stream headed for the default output: it can start in its own input (see
    /// [`Router::claim_early`]).
    early: bool,
    /// Taken for AcerUltraX by [`Router::on_app_format`], before being linked anywhere.
    claimed_early: bool,
    positions: Option<Vec<u32>>,
    wanted: bool,
    _proxy: Node,
    _listener: NodeListener,
}

/// A stream with its own hidden sink and filter port group; without an app, one kept ready.
struct Slot {
    index: usize,
    app: Option<u32>,
    channels: Vec<&'static str>,
    sink_name: String,
    sink_id: Option<u32>,
    _sink: Node,
    links: Vec<Option<Link>>,
    moved: bool,
    created: Instant,
}

/// What the daemon reports about itself.
#[derive(serde::Serialize, Clone, Default)]
pub struct Status {
    pub device: Option<String>,
    pub device_name: Option<String>,
    pub kind: Option<Kind>,
    pub profile: Option<String>,
    pub mode: Option<String>,
    pub enabled: bool,
    pub bypass: bool,
    pub compare: bool,
    pub rate: u32,
    pub streams: usize,
    /// Headphone model in use ("Brand Name"), if any.
    pub headphone: Option<String>,
    /// Spatial sound: available at all, and on for this output.
    pub spatial_available: bool,
    pub spatial: bool,
    pub spatial_power_saving: bool,
}

#[derive(serde::Serialize)]
pub struct DeviceStatus {
    pub device: String,
    pub device_name: String,
    pub kind: Kind,
    pub profile: String,
    pub active: bool,
    pub present: bool,
    /// Whether this output takes a headphone model, the chosen model id (`default`: Acer's), and whether it
    /// was recognised automatically.
    pub wearable: bool,
    pub headphone: Option<String>,
    pub headphone_auto: bool,
}

pub struct Router {
    core: CoreRc,
    registry: RegistryRc,
    filter: Filter,
    shared: Arc<Shared>,
    lib: &'static dts_eagle::Library,
    oem: Arc<dts_eagle::OemConfig>,
    opts: Options,
    pub config: Config,
    state: State,

    _sink: Node,
    _sink_listener: Option<NodeListener>,
    sink_id: Option<u32>,
    filter_id: Option<u32>,
    nodes: HashMap<u32, NodeInfo>,
    /// Form factor and icon name of each PipeWire device.
    devices: HashMap<u32, String>,
    ports: HashMap<u32, PortInfo>,
    links: HashMap<u32, (u32, u32)>,
    apps: HashMap<u32, App>,
    slots: Vec<Slot>,
    group0_links: Vec<Option<Link>>,
    metadata: Option<(Metadata, MetadataListener)>,
    /// Streams the default metadata gives a target (e.g. WirePlumber restoring an app's output).
    targets: HashSet<u32>,
    /// AcerUltraX is the default output.
    default_ours: bool,

    target: Option<u32>,
    out_links: Vec<(Link, LinkListener)>,
    /// A link to the device stopped after running (nothing plays; the device may be suspended next). Set from
    /// link events.
    out_stale: Rc<Cell<bool>>,
    /// Port groups of released streams, removed from the filter once their links are gone.
    retired: Vec<(crate::filter::Group, Instant)>,
    /// Ready inputs made so far (they are numbered).
    spares: u32,
    /// Profile in effect and the (mode, tuning, rate) its engines were built for.
    profile: Option<Profile>,
    built: Option<Build>,
    catalog: importer::Catalog,
    /// The spatial encoder's folder and module, when it was imported.
    spatial: Option<(std::path::PathBuf, &'static pe_loader::Module)>,
    dirty_since: Option<Instant>,
    volume_quiet_until: Instant,
    /// Until when the common input is held silent (see [`Router::add_slot`]).
    hold_common_until: Option<Instant>,
    /// Prevents a sink volume set before the device's is known from reaching the device.
    volume_synced: bool,
    ticks: u64,
}

pub type RouterRc = Rc<RefCell<Router>>;

/// What the engines in use were built for.
#[derive(Clone, PartialEq, Debug)]
struct Build {
    mode: dts_eagle::Mode,
    tuning: dts_eagle::Output,
    rate: u32,
    /// Headphone EQ files (48k, 44k) replacing Acer's default headphone EQ.
    eq: Option<[String; 2]>,
    tweaks: crate::config::Tweaks,
    /// Spatial renderer and preset file, if spatial sound is on.
    spatial: Option<(dts_spatial::Render, String)>,
}

fn prop<'a>(d: &'a DictRef, k: &str) -> Option<&'a str> {
    d.get(k)
}

impl Router {
    pub fn new(
        core: CoreRc,
        loop_: &pipewire::loop_::Loop,
        lib: &'static dts_eagle::Library,
        oem: Arc<dts_eagle::OemConfig>,
        catalog: importer::Catalog,
        spatial: Option<(std::path::PathBuf, &'static pe_loader::Module)>,
        opts: Options,
    ) -> anyhow::Result<RouterRc> {
        let registry = core.get_registry_rc()?;
        let shared = Arc::new(Shared::default());
        let mut filter = Filter::new(loop_, lib, shared.clone());
        // Group 0: the visible sink's monitor, for streams not moved (yet).
        let g0 = filter.add_group(0, SHARED_ID, &["FL", "FR"]);
        shared.groups.lock().unwrap().push(g0);
        filter.connect();

        let sink: Node = core.create_object(
            "adapter",
            &properties! {
                "factory.name" => "support.null-audio-sink",
                "node.name" => SINK_NAME,
                "node.description" => "AcerUltraX",
                "media.class" => "Audio/Sink",
                "audio.channels" => "2",
                "audio.position" => "[FL,FR]",
                "monitor.channel-volumes" => "false",
                "priority.session" => "2000",
                "priority.driver" => "1",
                "node.virtual" => "true",
                "object.linger" => "false",
            },
        )?;
        let state = State::load();
        let router = Rc::new(RefCell::new(Router {
            core,
            registry,
            filter,
            shared,
            lib,
            oem,
            opts,
            config: Config::load(),
            state,
            _sink: sink,
            _sink_listener: None,
            sink_id: None,
            filter_id: None,
            nodes: HashMap::new(),
            devices: HashMap::new(),
            ports: HashMap::new(),
            links: HashMap::new(),
            apps: HashMap::new(),
            slots: Vec::new(),
            group0_links: vec![None, None],
            metadata: None,
            targets: HashSet::new(),
            default_ours: false,
            target: None,
            out_links: Vec::new(),
            out_stale: Rc::default(),
            retired: Vec::new(),
            spares: 0,
            profile: None,
            built: None,
            catalog,
            spatial,
            dirty_since: None,
            volume_quiet_until: Instant::now(),
            hold_common_until: None,
            volume_synced: false,
            ticks: 0,
        }));
        let weak = Rc::downgrade(&router);
        let listener = router
            .borrow()
            ._sink
            .add_listener_local()
            .param(move |_, id, _, _, pod| {
                if id == ParamType::Props {
                    if let (Some(r), Some(pod)) = (weak.upgrade(), pod) {
                        r.borrow_mut().on_sink_props(pod);
                    }
                }
            })
            .register();
        router.borrow()._sink.subscribe_params(&[ParamType::Props]);
        router.borrow_mut()._sink_listener = Some(listener);
        Ok(router)
    }

    pub fn registry(&self) -> RegistryRc {
        self.registry.clone()
    }

    // ---------------------------------------------------------------- registry

    pub fn on_global(this: &RouterRc, g: &GlobalObject<&DictRef>) {
        let Some(props) = g.props else { return };
        let mut r = this.borrow_mut();
        match g.type_ {
            ObjectType::Node => r.on_node(this, g, props),
            ObjectType::Port => {
                let (Some(node), Some(name), Some(dir)) =
                    (prop(props, "node.id"), prop(props, "port.name"), prop(props, "port.direction"))
                else {
                    return;
                };
                let node = node.parse().unwrap_or(0);
                r.ports.insert(g.id, PortInfo { node, output: dir == "out", name: name.into() });
                r.try_link_all();
                if Some(node) == r.target || Some(node) == r.filter_id {
                    r.link_output();
                }
            }
            ObjectType::Link => {
                let (Some(o), Some(i)) = (prop(props, "link.output.node"), prop(props, "link.input.node")) else {
                    return;
                };
                let (o, i) = (o.parse().unwrap_or(0), i.parse().unwrap_or(0));
                r.links.insert(g.id, (o, i));
                if r.claims(i) {
                    r.want(o);
                } else if let Some(k) =
                    r.slots.iter().position(|s| s.app == Some(o) && s.sink_id.is_some_and(|id| id != i))
                {
                    // Started in its own input but sent elsewhere (e.g. its remembered output): leave the target that
                    // sent it there alone.
                    if let Some(a) = r.apps.get_mut(&o) {
                        a.wanted = false;
                    }
                    r.slots[k].moved = false;
                    r.release(k);
                }
            }
            ObjectType::Device => {
                // e.g. "headset audio-headset-bluetooth" or "speaker audio-speakers-bluetooth"
                let what = [prop(props, "device.form-factor"), prop(props, "device.icon-name")]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase();
                r.devices.insert(g.id, what);
                r.mark_dirty();
            }
            ObjectType::Metadata if prop(props, "metadata.name") == Some("default") && r.metadata.is_none() => {
                let Ok(md) = r.registry.bind::<Metadata, _>(g) else { return };
                let weak = Rc::downgrade(this);
                let listener = md
                    .add_listener_local()
                    .property(move |subject, key, _, value| {
                        let Some(r) = weak.upgrade() else { return 0 };
                        let Ok(mut r) = r.try_borrow_mut() else { return 0 };
                        match (subject, key) {
                            (0, Some(DEFAULT_KEY)) => r.on_default_changed(value),
                            (s, Some("target.object" | "target.node")) if s != 0 => {
                                if value.is_some() {
                                    r.targets.insert(s);
                                } else {
                                    r.targets.remove(&s);
                                }
                            }
                            _ => {}
                        }
                        0
                    })
                    .register();
                r.metadata = Some((md, listener));
                if !r.opts.no_default {
                    r.set_default(SINK_NAME);
                }
            }
            _ => {}
        }
    }

    fn on_node(&mut self, this: &RouterRc, g: &GlobalObject<&DictRef>, props: &DictRef) {
        let name = prop(props, "node.name").unwrap_or("").to_string();
        let class = prop(props, "media.class").unwrap_or("").to_string();
        if name == SINK_NAME {
            self.sink_id = Some(g.id);
            self.try_link_all();
            let ids: Vec<_> = self.links.values().copied().collect();
            for (o, i) in ids {
                if self.claims(i) {
                    self.want(o);
                }
            }
            return;
        }
        if name == FILTER_NAME {
            self.filter_id = Some(g.id);
            self.try_link_all();
            return;
        }
        if let Some(slot) = self.slots.iter_mut().find(|s| s.sink_name == name) {
            slot.sink_id = Some(g.id);
            self.try_link_all();
            return;
        }
        if class == "Stream/Output/Audio" {
            let Ok(proxy) = self.registry.bind::<Node, _>(g) else { return };
            let (id, weak) = (g.id, Rc::downgrade(this));
            let listener = proxy
                .add_listener_local()
                .info({
                    let weak = weak.clone();
                    move |info| {
                        if let (Some(r), Some(props)) = (weak.upgrade(), info.props()) {
                            r.borrow_mut().on_app_info(id, props);
                        }
                    }
                })
                .param(move |_, pid, _, _, pod| {
                    // The format a stream offers is known before it is linked.
                    if pid != ParamType::Format && pid != ParamType::EnumFormat {
                        return;
                    }
                    let (Some(r), Some(pod)) = (weak.upgrade(), pod) else { return };
                    if let Some(pos) = parse_positions(pod) {
                        r.borrow_mut().on_app_format(id, pos);
                    }
                })
                .register();
            proxy.subscribe_params(&[ParamType::Format, ParamType::EnumFormat]);
            let serial = prop(props, "object.serial").and_then(|s| s.parse().ok()).unwrap_or(g.id);
            self.apps.insert(
                g.id,
                App {
                    serial,
                    dont_move: false,
                    info: false,
                    early: false,
                    claimed_early: false,
                    positions: None,
                    wanted: false,
                    _proxy: proxy,
                    _listener: listener,
                },
            );
            return;
        }
        if class == "Audio/Sink" && !name.starts_with("acerultrax") {
            let api = prop(props, "device.api").unwrap_or("");
            self.nodes.insert(
                g.id,
                NodeInfo {
                    description: prop(props, "node.description").unwrap_or(&name).to_string(),
                    name,
                    class,
                    priority: prop(props, "priority.session").and_then(|p| p.parse().ok()).unwrap_or(0),
                    bluetooth: api == "bluez5",
                    usb: prop(props, "device.bus") == Some("usb"),
                    hardware: prop(props, "device.id").is_some(),
                    device: prop(props, "device.id").and_then(|d| d.parse().ok()),
                },
            );
            self.mark_dirty();
        }
    }

    pub fn on_global_remove(this: &RouterRc, id: u32) {
        let mut r = this.borrow_mut();
        if r.apps.remove(&id).is_some() {
            if let Some(i) = r.slots.iter().position(|s| s.app == Some(id)) {
                r.slots[i].moved = false; // the stream is gone, no target to clear
                r.release(i);
            }
        }
        r.targets.remove(&id);
        r.ports.remove(&id);
        r.links.remove(&id);
        r.devices.remove(&id);
        if r.nodes.remove(&id).is_some() {
            if r.target == Some(id) {
                r.target = None;
                r.out_links.clear();
            }
            r.mark_dirty();
        }
        for s in &mut r.slots {
            if s.sink_id == Some(id) {
                s.sink_id = None;
            }
        }
        if r.sink_id == Some(id) {
            r.sink_id = None;
        }
    }

    // ---------------------------------------------------------------- streams

    /// Streams playing into AcerUltraX or straight into the device it uses get DTS (as all audio to the
    /// endpoint does on Windows).
    fn claims(&self, input_node: u32) -> bool {
        input_node != 0 && (Some(input_node) == self.sink_id || Some(input_node) == self.target)
    }

    fn want(&mut self, app: u32) {
        let Some(a) = self.apps.get_mut(&app) else { return };
        if a.wanted {
            return;
        }
        a.wanted = true;
        if !a.dont_move && a.positions.is_some() {
            self.add_slot(app);
        }
    }

    fn on_app_info(&mut self, app: u32, props: &DictRef) {
        let Some(a) = self.apps.get_mut(&app) else { return };
        if a.info {
            return;
        }
        a.info = true;
        a.dont_move = prop(props, "node.dont-move") == Some("true");
        let targeted = prop(props, "target.object").or(prop(props, "node.target")).is_some_and(|t| t != SINK_NAME)
            || prop(props, "node.autoconnect") == Some("false");
        // PulseAudio clients are not answered until linked to a real sink, so they cannot start in an input.
        a.early = !targeted && !a.dont_move && prop(props, "client.api") != Some("pipewire-pulse");
        self.claim_early(app);
    }

    fn on_app_format(&mut self, app: u32, positions: Vec<u32>) {
        let Some(a) = self.apps.get_mut(&app) else { return };
        let first = a.positions.is_none();
        a.positions = Some(positions);
        if first && a.wanted && !a.dont_move {
            self.add_slot(app);
        }
        self.claim_early(app);
    }

    /// A native stream bound for the default output goes to its own input before it plays: moving a stream that
    /// plays loses the block in transit (about 20 ms at its start). Needs its properties and format.
    fn claim_early(&mut self, app: u32) {
        let ours = self.default_ours && !self.targets.contains(&app);
        let Some(a) = self.apps.get_mut(&app) else { return };
        if ours && a.info && a.early && !a.wanted && a.positions.is_some() {
            (a.wanted, a.claimed_early) = (true, true);
            self.add_slot(app);
        }
    }

    fn add_slot(&mut self, app: u32) {
        if self.slots.iter().any(|s| s.app == Some(app)) {
            return;
        }
        let a = &self.apps[&app];
        let (serial, early) = (a.serial, a.claimed_early);
        let channels = layout::group_layout(a.positions.as_deref().unwrap_or(&[]));
        // A PulseAudio app's first two blocks reach AcerUltraX before it can be moved, through the common input:
        // downmixed to stereo and through the common stereo engine. For 5.1/7.1 that sounds different from
        // its own engine and the switch clicks, so the common input is silent until the stream has moved (it
        // then starts about 40 ms in, in its own engine, as on Windows). Stereo and mono sound the same either way.
        if channels.len() > 2 && !early {
            self.hold_common_until = Some(Instant::now() + Duration::from_millis(400));
            self.shared.hold_common.store(true, Ordering::Relaxed);
        }
        // Mono goes into a stereo input on both sides, as its stream engine would take it anyway.
        let fits = |s: &Slot| s.channels == channels || (channels == ["MONO"] && s.channels == ["FL", "FR"]);
        if let Some(i) = self.slots.iter().position(|s| s.app.is_none() && fits(s)) {
            let index = self.slots[i].index;
            if let Some(g) = self.shared.groups.lock().unwrap().iter_mut().find(|g| g.index == index) {
                g.id = serial;
            }
            self.slots[i].app = Some(app);
            log::info!("stream {app} ({} ch) gets its own DTS engine", channels.len());
            self.move_app(i);
            return;
        }
        if self.new_slot(Some(app), serial, channels.clone()) {
            log::info!("stream {app} ({} ch) gets its own DTS engine", channels.len());
        }
    }

    /// Keep a stereo input ready while AcerUltraX is the default output, so a new stream can be sent to its own
    /// input at once (making one takes several round trips, and the session manager may link it first).
    fn ensure_spare(&mut self) {
        if !self.default_ours || self.filter_id.is_none() || self.slots.iter().any(|s| s.app.is_none()) {
            return;
        }
        self.spares += 1;
        self.new_slot(None, u32::MAX - self.spares, vec!["FL", "FR"]);
    }

    fn new_slot(&mut self, app: Option<u32>, serial: u32, channels: Vec<&'static str>) -> bool {
        let taken =
            |i: usize| self.slots.iter().any(|s| s.index == i) || self.retired.iter().any(|(g, _)| g.index == i);
        let Some(index) = (1..MAX_GROUPS).find(|&i| !taken(i)) else {
            if let Some(app) = app {
                log::warn!("too many streams; stream {app} shares the common input");
            }
            return false;
        };
        let sink_name = match app {
            Some(_) => format!("{IN_PREFIX}{serial}"),
            None => format!("{IN_PREFIX}ready{}", self.spares),
        };
        let sink: Node = match self.core.create_object(
            "adapter",
            &properties! {
                "factory.name" => "support.null-audio-sink",
                "node.name" => sink_name.as_str(),
                "node.description" => format!("AcerUltraX input {serial}").as_str(),
                "media.class" => "Audio/Sink/Internal",
                "audio.channels" => channels.len().to_string().as_str(),
                "audio.position" => format!("[{}]", channels.join(",")).as_str(),
                "monitor.channel-volumes" => "false",
                "priority.driver" => "1",
                "node.virtual" => "true",
                "object.linger" => "false",
            },
        ) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("cannot create an input: {e}");
                return false;
            }
        };
        let group = self.filter.add_group(index, serial, &channels);
        self.shared.groups.lock().unwrap().push(group);
        self.slots.push(Slot {
            index,
            app,
            links: (0..channels.len()).map(|_| None).collect(),
            channels,
            sink_name,
            sink_id: None,
            _sink: sink,
            moved: false,
            created: Instant::now(),
        });
        true
    }

    fn move_app(&mut self, i: usize) {
        let Some(app) = self.slots[i].app else { return };
        // Taken early, but since given a target of its own (e.g. WirePlumber restoring the app's output): let it go.
        if self.targets.contains(&app) && self.apps.get(&app).is_some_and(|a| a.claimed_early) {
            if let Some(a) = self.apps.get_mut(&app) {
                (a.wanted, a.claimed_early) = (false, false);
            }
            self.release(i);
            return;
        }
        let s = &mut self.slots[i];
        let Some((md, _)) = &self.metadata else { return };
        if s.moved || s.sink_id.is_none() {
            return;
        }
        md.set_property(app, "target.object", Some("Spa:Id"), Some(&s.sink_name));
        s.moved = true;
    }

    fn release(&mut self, i: usize) {
        let s = self.slots.remove(i);
        let group = {
            let mut groups = self.shared.groups.lock().unwrap();
            groups.iter().position(|g| g.index == s.index).map(|p| groups.remove(p))
        };
        if let Some(app) = s.app.filter(|a| s.moved && self.apps.contains_key(a)) {
            if let Some((md, _)) = &self.metadata {
                md.set_property(app, "target.object", None, None);
            }
        }
        // The input and its links go first; the ports a moment later (see `tick`): ports removed while still
        // linked make PipeWire fail on them, which puts the filter in an error state once it goes idle.
        drop(s);
        if let Some(g) = group {
            self.retired.push((g, Instant::now()));
        }
    }

    fn port(&self, node: u32, output: bool, name: &str) -> Option<u32> {
        self.ports.iter().find(|(_, p)| p.node == node && p.output == output && p.name == name).map(|(id, _)| *id)
    }

    fn link(&self, out_node: u32, out_port: u32, in_node: u32, in_port: u32, passive: bool) -> Option<Link> {
        let p = properties! {
            "link.output.node" => out_node.to_string().as_str(),
            "link.output.port" => out_port.to_string().as_str(),
            "link.input.node" => in_node.to_string().as_str(),
            "link.input.port" => in_port.to_string().as_str(),
            "link.passive" => if passive { "true" } else { "false" },
            "object.linger" => "false",
        };
        self.core.create_object::<Link>("link-factory", &p).map_err(|e| log::warn!("link failed: {e}")).ok()
    }

    /// Link every input group whose ports are known.
    fn try_link_all(&mut self) {
        let Some(f) = self.filter_id else { return };
        if let Some(sink) = self.sink_id {
            for (c, ch) in ["FL", "FR"].iter().enumerate() {
                if self.group0_links[c].is_none() {
                    if let (Some(o), Some(i)) =
                        (self.port(sink, true, &format!("monitor_{ch}")), self.port(f, false, &format!("g0_{ch}")))
                    {
                        // Group 0 only carries streams in transit: do not keep the graph running for it.
                        self.group0_links[c] = self.link(sink, o, f, i, true);
                    }
                }
            }
        }
        for k in 0..self.slots.len() {
            let Some(sink) = self.slots[k].sink_id else { continue };
            for c in 0..self.slots[k].channels.len() {
                if self.slots[k].links[c].is_some() {
                    continue;
                }
                let ch = self.slots[k].channels[c];
                let (o, i) = (
                    self.port(sink, true, &format!("monitor_{ch}")),
                    self.port(f, false, &format!("g{}_{ch}", self.slots[k].index)),
                );
                if let (Some(o), Some(i)) = (o, i) {
                    self.slots[k].links[c] = self.link(sink, o, f, i, false);
                }
            }
        }
        // Only once linked: moved into an input not linked yet, a stream's first block went nowhere. Last first,
        // as a move may release its slot.
        for k in (0..self.slots.len()).rev() {
            if !self.slots[k].moved && self.slots[k].links.iter().all(Option::is_some) {
                self.move_app(k);
            }
        }
    }

    /// Streams whose input could not be wired up within 3 s go back to the common input instead of silence.
    pub fn check_slots(&mut self) {
        while let Some(i) = self
            .slots
            .iter()
            .position(|s| s.created.elapsed() > Duration::from_secs(3) && s.links.iter().any(Option::is_none))
        {
            if let Some(app) = self.slots[i].app {
                log::warn!("stream {app}: its input is not connected after 3 s; using the common input");
                if let Some(a) = self.apps.get_mut(&app) {
                    a.dont_move = true;
                }
            }
            self.release(i);
        }
    }

    // ---------------------------------------------------------------- output device

    fn mark_dirty(&mut self) {
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    fn set_default(&self, name: &str) {
        if let Some((md, _)) = &self.metadata {
            md.set_property(0, DEFAULT_KEY, Some("Spa:String:JSON"), Some(&format!("{{\"name\":\"{name}\"}}")));
        }
    }

    fn on_default_changed(&mut self, value: Option<&str>) {
        let Some(name) = value.and_then(json_name) else { return };
        self.default_ours = name == SINK_NAME;
        if self.default_ours || self.opts.no_default {
            return;
        }
        if self.nodes.values().any(|n| n.name == name && n.hardware) {
            log::info!("user picked {name}");
            self.state.chose(&name);
            self.state.save();
            self.set_default(SINK_NAME);
            self.mark_dirty();
        }
    }

    fn kind(&self, n: &NodeInfo) -> Kind {
        // shortcut: wired outputs are classified by name, which fits laptops whose speakers and headphones are
        // separate sinks (UCM); laptops switching ports inside one sink need the active route checked instead.
        let lname = n.name.to_lowercase();
        let device = n.device.and_then(|d| self.devices.get(&d)).map_or("", |s| s.as_str());
        let worn = ["headphone", "headset", "hands-free", "handsfree"].iter().any(|w| device.contains(w));
        if n.bluetooth {
            // Unknown Bluetooth devices count as headphones, by far the most common case on a laptop.
            let speaker = ["speaker", "portable", "car", "hifi"].iter().any(|w| device.contains(w));
            if speaker && !worn { Kind::BluetoothSpeaker } else { Kind::Bluetooth }
        } else if worn || lname.contains("headphone") || lname.contains("headset") {
            Kind::Headphones
        } else if lname.starts_with("alsa_output.pci") && (lname.contains("speaker") || lname.contains("analog")) {
            Kind::Speakers
        } else {
            Kind::Other
        }
    }

    /// Pick the device (debounced: a headphone plug produces a burst of changes) and apply its profile.
    pub fn tick(&mut self) {
        self.shared.empty_trash();
        self.check_slots();
        self.ensure_spare();
        while self.retired.first().is_some_and(|(_, t)| t.elapsed() > Duration::from_millis(500)) {
            let (g, _) = self.retired.remove(0);
            self.filter.remove_group(g);
        }
        if self.out_stale.replace(false) {
            // Nothing plays any more. Once the device is suspended, the (passive) links that ran do not get it
            // going again, but links made while it is idle or suspended do: link it afresh, while it is quiet.
            self.out_links.clear();
            self.link_output();
        }
        if self.hold_common_until.is_some_and(|t| t <= Instant::now()) {
            self.hold_common_until = None;
            self.shared.hold_common.store(false, Ordering::Relaxed);
        }
        self.ticks += 1;
        if self.ticks % 300 == 0 {
            // Engines of idle apps are freed on the audio thread; hand that memory back to the system.
            unsafe { libc::malloc_trim(0) };
        }
        let rate = self.shared.rate.load(Ordering::Relaxed);
        if let Some(b) = self.built.clone() {
            if rate != 0 && rate != b.rate {
                self.build(Build { rate, ..b });
            }
        }
        match self.dirty_since {
            Some(t) if t.elapsed() >= Duration::from_millis(300) => self.dirty_since = None,
            _ => return,
        }
        let forced = self.opts.target.is_some();
        let candidates: Vec<(u32, &NodeInfo)> = self
            .nodes
            .iter()
            .filter(|(_, n)| (forced || n.hardware) && n.class == "Audio/Sink")
            .map(|(i, n)| (*i, n))
            .collect();
        let pick = match &self.opts.target {
            Some(t) => candidates.iter().find(|(_, n)| &n.name == t),
            None => self
                .state
                .history
                .iter()
                .find_map(|h| candidates.iter().find(|(_, n)| &n.name == h))
                .or_else(|| candidates.iter().max_by_key(|(_, n)| n.priority)),
        }
        .map(|(id, _)| *id);
        if pick != self.target {
            self.target = pick;
            self.out_links.clear();
            self.volume_synced = false;
            if let Some(n) = pick.and_then(|id| self.nodes.get(&id)) {
                log::info!("output: {} ({})", n.description, n.name);
            }
            self.link_output();
            self.sync_volume_from_device();
        }
        self.apply_profile();
    }

    fn link_output(&mut self) {
        let (Some(f), Some(t)) = (self.filter_id, self.target) else { return };
        if !self.out_links.is_empty() {
            return;
        }
        let playback: Vec<(u32, String)> =
            self.ports.iter().filter(|(_, p)| p.node == t && !p.output).map(|(id, p)| (*id, p.name.clone())).collect();
        if playback.is_empty() {
            return;
        }
        // Link all at once, or not yet: a partly linked device is never revisited.
        if ["FL", "FR", "MONO"].iter().any(|ch| self.port(f, true, &format!("output_{ch}")).is_none()) {
            return;
        }
        let speakers = self.nodes.get(&t).is_some_and(|n| self.kind(n) == Kind::Speakers);
        let stereo = ["playback_FL", "playback_FR"].iter().all(|c| playback.iter().any(|(_, n)| n == c));
        for (i, name) in &playback {
            let src = layout::output_source(name.strip_prefix("playback_").unwrap_or(name), stereo, speakers);
            let Some(o) = src.and_then(|ch| self.port(f, true, &format!("output_{ch}"))) else { continue };
            if let Some(l) = self.link(f, o, t, *i, true) {
                let (stale, was_active) = (self.out_stale.clone(), Cell::new(false));
                let listener = l
                    .add_listener_local()
                    .info(move |info| match info.state() {
                        LinkState::Active => was_active.set(true),
                        LinkState::Paused | LinkState::Init if was_active.get() => stale.set(true),
                        _ => {}
                    })
                    .register();
                self.out_links.push((l, listener));
            }
        }
    }

    fn apply_profile(&mut self) {
        self.remember_bluetooth();
        let Some(n) = self.target.and_then(|t| self.nodes.get(&t)) else { return };
        let kind = self.kind(n);
        let profile = self.config.profile_for(&n.name, kind);
        self.shared.bypass.store(!self.config.enabled || profile.dts_mode().is_none(), Ordering::Relaxed);
        if let Some(mode) = profile.dts_mode() {
            let rate = match self.shared.rate.load(Ordering::Relaxed) {
                0 => 48000,
                r => r,
            };
            let eq = self.headphone_for(n, kind).map(|(h, route)| [h.eq_file(route, "48k"), h.eq_file(route, "44k")]);
            let spatial = self.spatial_for(n, kind);
            let b = Build { mode, tuning: kind.tuning(), rate, eq, tweaks: profile.tweaks.clone(), spatial };
            if self.built.as_ref() != Some(&b) {
                self.build(b);
            }
        }
        self.profile = Some(profile);
    }

    /// Spatial sound on an output: the renderer (speakers or headphones) and its preset, if switched on there.
    fn spatial_for(&self, n: &NodeInfo, kind: Kind) -> Option<(dts_spatial::Render, String)> {
        self.spatial.as_ref()?;
        if !self.config.spatial.get(&self.headphone_key(n, kind)).copied().unwrap_or(false) {
            return None;
        }
        let headphones = matches!(kind, Kind::Headphones | Kind::Bluetooth) || (kind == Kind::Other && n.usb);
        let power = self.config.spatial_power_saving;
        // The presets DTS's app uses: "Best sound" (and "Balanced") or "Best power".
        let (render, preset) = match (headphones, power) {
            (false, false) => (dts_spatial::Render::Speakers, "02-SPAC-HqHeightAndHgNf_SD0_Spk_Normal_v4_RC2.SPAC"),
            (false, true) => (dts_spatial::Render::Speakers, "04-SPAC-BasicSDContent_SD0_Spk_LowPower_v4_RC2.SPAC"),
            (true, false) => (dts_spatial::Render::Headphones, "02-SPAC-HqHeightAndHgNf_SD1_Hp_Normal_v4_RC2.SPAC"),
            (true, true) => (dts_spatial::Render::Headphones, "06-SPAC-BasicSDContent_SD1_Hp_LowPower_v4_RC2.SPAC"),
        };
        Some((render, preset.to_string()))
    }

    /// Spatial sound on or off for `device` (default: the current output).
    pub fn set_spatial(&mut self, on: bool, device: Option<&str>) -> Result<(), String> {
        if self.spatial.is_none() {
            return Err("spatial sound is not set up (set up Acer's \"XPERI DTS Utility\" download)".into());
        }
        let key = match device {
            Some(d) => d.to_string(),
            None => {
                let n = self.target.and_then(|t| self.nodes.get(&t)).ok_or("no output device")?;
                self.headphone_key(n, self.kind(n))
            }
        };
        self.config.spatial.insert(key, on);
        self.config.save();
        self.apply_profile();
        Ok(())
    }

    pub fn set_spatial_power_saving(&mut self, on: bool) {
        self.config.spatial_power_saving = on;
        self.config.save();
        self.apply_profile();
    }

    /// Settings key of an output's headphone model.
    fn headphone_key(&self, n: &NodeInfo, kind: Kind) -> String {
        if kind == Kind::Headphones { "@headphones".into() } else { n.name.clone() }
    }

    /// The headphone model in use on an output and the connection whose EQ to use, if any: the user's choice,
    /// else the model whose Bluetooth/USB name matches the device's.
    fn headphone_for(&self, n: &NodeInfo, kind: Kind) -> Option<(&importer::Headphone, &'static str)> {
        let route = match kind {
            Kind::Headphones => "line-out",
            Kind::Bluetooth => "bluetooth",
            Kind::Other if n.usb => "usb",
            _ => return None,
        };
        let h = match self.config.headphones.get(&self.headphone_key(n, kind)).map(String::as_str) {
            Some("default") => None,
            Some(id) => self.catalog.get(id),
            None if kind != Kind::Headphones => self.catalog.match_device(&n.description),
            None => None,
        }?;
        let r = h.route_for(route)?;
        Some((h, ["line-out", "bluetooth", "usb"].into_iter().find(|x| *x == r).unwrap_or("line-out")))
    }

    pub fn headphones(&self) -> &[importer::Headphone] {
        &self.catalog.headphones
    }

    /// Use headphone model `id` (None: automatic / Acer's default; "default": always Acer's default) on
    /// `device` (as in [`Router::devices`]; default: the current output).
    pub fn set_headphone(&mut self, id: Option<&str>, device: Option<&str>) -> Result<(), String> {
        if let Some(id) = id.filter(|i| *i != "default") {
            self.catalog.get(id).ok_or_else(|| format!("no headphone model {id:?}"))?;
        }
        let key = match device {
            Some(d) => d.to_string(),
            None => {
                let n = self.target.and_then(|t| self.nodes.get(&t)).ok_or("no output device")?;
                self.headphone_key(n, self.kind(n))
            }
        };
        match id {
            Some(i) => self.config.headphones.insert(key, i.to_string()),
            None => self.config.headphones.remove(&key),
        };
        self.config.save();
        self.apply_profile();
        Ok(())
    }

    fn remember_bluetooth(&mut self) {
        let seen: Vec<(String, (String, Kind))> = self
            .nodes
            .values()
            .filter(|n| n.bluetooth)
            .map(|n| (n.name.clone(), (n.description.clone(), self.kind(n))))
            .filter(|(k, v)| self.state.bluetooth.get(k) != Some(v))
            .collect();
        if !seen.is_empty() {
            self.state.bluetooth.extend(seen);
            self.state.save();
        }
    }

    /// The engine settings for a build: the mode's factory tuning, the headphone model's EQ in place of Acer's
    /// default (as Windows does when a model is picked), then the profile's adjustments.
    fn tuning_for(&self, b: &Build) -> Option<dts_eagle::ModeTuning> {
        let mut t = self.oem.tuning(b.mode, b.tuning).cloned()?;
        if let Some([e48, e44]) = &b.eq {
            for s in &mut t.settings {
                match s.name.as_str() {
                    "Eagle-HPEQ Blob 48k" => s.value = e48.clone(),
                    "Eagle-HPEQ Blob 44k" => s.value = e44.clone(),
                    _ => {}
                }
            }
        }
        b.tweaks.apply(&mut t);
        Some(t)
    }

    pub fn lib(&self) -> &'static dts_eagle::Library {
        self.lib
    }

    /// What to measure for the response curve of `profile` on the current output: engine settings and rate,
    /// or `None` for plain sound (a flat line).
    #[allow(clippy::type_complexity)]
    pub fn response_job(
        &self,
        profile: &Profile,
    ) -> Result<Option<(dts_eagle::ModeTuning, u32, Option<[i32; 10]>)>, String> {
        let n = self.target.and_then(|t| self.nodes.get(&t)).ok_or("no output device")?;
        let kind = self.kind(n);
        let Some(mode) = profile.dts_mode() else { return Ok(None) };
        let eq = self.headphone_for(n, kind).map(|(h, route)| [h.eq_file(route, "48k"), h.eq_file(route, "44k")]);
        let rate = match self.shared.rate.load(Ordering::Relaxed) {
            r if dts_eagle::RATES.contains(&r) => r,
            _ => 48000,
        };
        let b = Build { mode, tuning: kind.tuning(), rate, eq, tweaks: profile.tweaks.clone(), spatial: None };
        Ok(Some((self.tuning_for(&b).ok_or("no tuning")?, rate, profile.tweaks.eq)))
    }

    /// A mode's factory values of the adjustable DTS controls on the current output (what an editor starts from).
    pub fn mode_defaults(&self, mode: &str) -> Result<crate::config::Tweaks, String> {
        let mode: dts_eagle::Mode = mode.parse().map_err(|_| format!("unknown mode {mode:?}"))?;
        let kind = self.target.and_then(|t| self.nodes.get(&t)).map_or(Kind::Speakers, |n| self.kind(n));
        let t = self.oem.tuning(mode, kind.tuning()).ok_or("no tuning")?;
        let get = |loc, name: &str| t.engine_settings(loc).into_iter().rev().find(|(n, _)| n == name).map(|(_, v)| v);
        let int = |loc, name: &str| get(loc, name).and_then(|v| v.parse::<i32>().ok());
        let on = |loc, name: &str| int(loc, name).map(|v| v != 0);
        use dts_eagle::Location::{Mfx, Sfx};
        let level = |loc, enable: &str, name: &str| match on(loc, enable) {
            Some(false) => Some(0),
            _ => int(loc, name),
        };
        Ok(crate::config::Tweaks {
            eq: None,
            bass: level(Mfx, "Eagle-TBHDX Enable", "Eagle-TBHDX Bass Level"),
            dialog: level(Sfx, "Eagle-DE Enable", "Eagle-DE Level"),
            surround: on(Sfx, "Eagle-I3DA Enable"),
            room: get(Sfx, "Eagle-I3DA Room Index")
                .and_then(|v| crate::config::ROOMS.iter().find(|(_, e)| *e == v).map(|(k, _)| k.to_string())),
            loudness: on(Sfx, "Eagle-LC Enable"),
        })
    }

    pub fn save_profile(&mut self, p: Profile) -> Result<(), String> {
        self.config.save_profile(p)?;
        self.config.save();
        self.apply_profile();
        Ok(())
    }

    pub fn delete_profile(&mut self, name: &str) -> Result<(), String> {
        if !self.config.delete_profile(name) {
            return Err(format!("no profile of yours named {name:?}"));
        }
        self.config.save();
        self.apply_profile();
        Ok(())
    }

    /// Prepare a mode's engines on a worker thread; the audio thread picks them up.
    fn build(&mut self, b: Build) {
        self.built = Some(b.clone());
        let (mode, tuning, rate) = (b.mode, b.tuning, b.rate);
        if !dts_eagle::RATES.contains(&rate) {
            log::warn!("DTS cannot run at {rate} Hz; passing audio through");
            return;
        }
        let Some(t) = self.tuning_for(&b) else { return };
        let eq = b.tweaks.eq;
        let spatial = match (&b.spatial, &self.spatial) {
            (Some((render, preset)), Some((dir, module))) => {
                let read = |f: &str| std::fs::read(dir.join(f)).map_err(|e| log::warn!("spatial preset {f}: {e}")).ok();
                match (read(preset), read("db_hpeq_disabled_.rdev")) {
                    (Some(p), Some(d)) => {
                        Some(audio_graph::Spatial { module, render: *render, preset: Arc::new(p), device: Arc::new(d) })
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        let (lib, shared) = (self.lib, self.shared.clone());
        let spares = self.shared.busy_streams.load(std::sync::atomic::Ordering::Relaxed);
        let spawned = std::thread::Builder::new().name("dts-build".into()).stack_size(8 << 20).spawn(move || {
            match audio_graph::Prepared::build(lib, &t, rate, eq, spatial) {
                Ok(mut p) => {
                    // Engines for the streams playing now, so they switch without work on the audio thread.
                    p.add_spares(lib, spares);
                    *shared.pending.lock().unwrap() = Some(p)
                }
                Err(e) => log::error!("cannot prepare {mode} ({tuning:?}, {rate} Hz): {e}"),
            }
        });
        if let Err(e) = spawned {
            log::error!("cannot start a worker thread: {e}");
        }
        log::info!(
            "mode {mode}, {tuning:?} tuning, {rate} Hz{}{}",
            b.eq.as_ref().map(|e| format!(", headphone EQ {}", e[0])).unwrap_or_default(),
            if b.tweaks.is_empty() { "" } else { ", adjusted" }
        );
        if let Some((_, preset)) = &b.spatial {
            log::info!("spatial sound on for 5.1/7.1 sound: {preset}");
        }
    }

    // ---------------------------------------------------------------- volume

    fn sync_volume_from_device(&mut self) {
        let (Some(t), Some(s)) = (self.target, self.sink_id) else { return };
        if let Some(v) = volume::get(t) {
            self.volume_quiet_until = Instant::now() + Duration::from_millis(500);
            volume::set(s, volume::from_device(v));
            self.volume_synced = true;
        }
    }

    fn on_sink_props(&mut self, pod: &Pod) {
        let Some(v) = volume::parse_props(pod) else { return };
        if !self.volume_synced || Instant::now() < self.volume_quiet_until {
            return;
        }
        if let Some(t) = self.target {
            volume::set_async(t, volume::to_device(v));
        }
    }

    // ---------------------------------------------------------------- control

    pub fn status(&self) -> Status {
        let n = self.target.and_then(|t| self.nodes.get(&t));
        Status {
            device: n.map(|n| n.name.clone()),
            device_name: n.map(|n| n.description.clone()),
            kind: n.map(|n| self.kind(n)),
            profile: self.profile.as_ref().map(|p| p.name.clone()),
            mode: self.profile.as_ref().and_then(|p| p.dts_mode()).map(|m| m.to_string()),
            enabled: self.config.enabled,
            bypass: self.shared.bypass.load(Ordering::Relaxed),
            compare: self.shared.match_loudness.load(Ordering::Relaxed),
            rate: self.shared.rate.load(Ordering::Relaxed),
            streams: self.slots.iter().filter(|s| s.app.is_some()).count(),
            headphone: n
                .and_then(|n| self.headphone_for(n, self.kind(n)))
                .map(|(h, _)| format!("{} {}", h.brand, h.name)),
            spatial_available: self.spatial.is_some(),
            spatial: n.is_some_and(|n| self.spatial_for(n, self.kind(n)).is_some()),
            spatial_power_saving: self.config.spatial_power_saving,
        }
    }

    /// The headphone part of a device entry (the other fields are filled by the caller).
    fn headphone_status(&self, key: &str, kind: Kind, description: &str, usb: bool) -> DeviceStatus {
        let wearable = matches!(kind, Kind::Headphones | Kind::Bluetooth) || (kind == Kind::Other && usb);
        let chosen = self.config.headphones.get(key).cloned();
        let auto = (wearable && chosen.is_none() && kind != Kind::Headphones)
            .then(|| self.catalog.match_device(description).map(|h| h.id.clone()))
            .flatten();
        DeviceStatus {
            device: String::new(),
            device_name: String::new(),
            kind,
            profile: String::new(),
            active: false,
            present: false,
            wearable,
            headphone_auto: auto.is_some(),
            headphone: chosen.or(auto),
        }
    }

    /// Output devices: the laptop's speakers and wired headphones (always listed), then every present or
    /// previously seen Bluetooth device, then other present outputs.
    pub fn devices(&self) -> Vec<DeviceStatus> {
        let active = self.target.and_then(|t| self.nodes.get(&t)).map(|n| (n.name.clone(), self.kind(n)));
        let mut v = Vec::new();
        for (kind, name) in [(Kind::Speakers, "Laptop speakers"), (Kind::Headphones, "Headphones (3.5 mm jack)")] {
            let present = self.nodes.values().find(|n| n.hardware && self.kind(n) == kind);
            v.push(DeviceStatus {
                device: format!("@{}", crate::config::kind_key(kind)),
                device_name: name.into(),
                kind,
                profile: self.config.profile_for(present.map_or("", |n| n.name.as_str()), kind).name,
                active: active.as_ref().is_some_and(|a| a.1 == kind),
                present: present.is_some(),
                ..self.headphone_status(&format!("@{}", crate::config::kind_key(kind)), kind, name, false)
            });
        }
        let mut rest: Vec<DeviceStatus> = self
            .nodes
            .values()
            .filter(|n| n.hardware && !matches!(self.kind(n), Kind::Speakers | Kind::Headphones))
            .map(|n| {
                let kind = self.kind(n);
                DeviceStatus {
                    device: n.name.clone(),
                    device_name: n.description.clone(),
                    kind,
                    profile: self.config.profile_for(&n.name, kind).name,
                    active: active.as_ref().is_some_and(|a| a.0 == n.name),
                    present: true,
                    ..self.headphone_status(&n.name, kind, &n.description, n.usb)
                }
            })
            .collect();
        for (name, (desc, kind)) in &self.state.bluetooth {
            if !rest.iter().any(|d| &d.device == name) {
                rest.push(DeviceStatus {
                    device: name.clone(),
                    device_name: desc.clone(),
                    kind: *kind,
                    profile: self.config.profile_for(name, *kind).name,
                    active: false,
                    present: false,
                    ..self.headphone_status(name, *kind, desc, false)
                });
            }
        }
        rest.sort_by_key(|d| (!matches!(d.kind, Kind::Bluetooth | Kind::BluetoothSpeaker), d.device_name.clone()));
        v.extend(rest);
        v
    }

    /// Whether the laptop's DTS tuning has the profile's mode (many have no Voice, Game2 or Game3).
    pub fn has_mode(&self, p: &Profile) -> bool {
        p.dts_mode().is_none_or(|m| self.oem.tuning(m, dts_eagle::Output::Speakers).is_some())
    }

    /// Use `profile` on `device` (default: the current one; `@speakers`/`@headphones`: the laptop's own outputs).
    pub fn set_profile(&mut self, profile: &str, device: Option<&str>) -> Result<(), String> {
        let p = self.config.profile(profile).ok_or_else(|| format!("no profile named {profile:?}"))?;
        if !self.has_mode(&p) {
            return Err(format!("this laptop's DTS tuning has no {} mode", p.mode));
        }
        let (name, kind) = match device {
            Some("@speakers") => (String::new(), Kind::Speakers),
            Some("@headphones") => (String::new(), Kind::Headphones),
            Some(d) => {
                let kind = self
                    .nodes
                    .values()
                    .find(|n| n.name == d)
                    .map(|n| self.kind(n))
                    .or_else(|| self.state.bluetooth.get(d).map(|b| b.1))
                    .unwrap_or(Kind::Other);
                (d.to_string(), kind)
            }
            None => {
                let n = self.target.and_then(|t| self.nodes.get(&t)).ok_or("no output device")?;
                (n.name.clone(), self.kind(n))
            }
        };
        self.config.set_device_profile(&name, kind, &p.name);
        self.config.save();
        self.apply_profile();
        Ok(())
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.config.enabled = on;
        self.config.save();
        self.apply_profile();
    }

    pub fn set_compare(&mut self, on: bool) {
        self.shared.match_loudness.store(on, Ordering::Relaxed);
    }

    pub fn reload_config(&mut self) {
        self.config = Config::load();
        self.apply_profile();
    }

    /// Hand apps and the default output back to the device before exiting.
    pub fn shutdown(&mut self) {
        while !self.slots.is_empty() {
            self.release(self.slots.len() - 1);
        }
        if !self.opts.no_default {
            if let Some(n) = self.target.and_then(|t| self.nodes.get(&t)) {
                self.set_default(&n.name.clone());
            }
        }
    }
}

/// `{"name":"x"}` -> x
fn json_name(v: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(v).ok()?.get("name")?.as_str().map(String::from)
}

/// Channel positions of a stream's Format param (unknown positions for unpositioned streams).
fn parse_positions(pod: &Pod) -> Option<Vec<u32>> {
    let mut info = pipewire::spa::param::audio::AudioInfoRaw::new();
    info.parse(pod).ok()?;
    let n = info.channels().min(64) as usize;
    if n == 0 {
        return None;
    }
    let unpositioned = info.flags().contains(pipewire::spa::param::audio::AudioInfoRawFlags::UNPOSITIONED);
    Some(info.position()[..n].iter().map(|&p| if unpositioned { 0 } else { p }).collect())
}

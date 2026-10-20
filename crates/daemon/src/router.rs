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

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use pipewire::core::CoreRc;
use pipewire::link::Link;
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
    positions: Option<Vec<u32>>,
    wanted: bool,
    _proxy: Node,
    _listener: NodeListener,
}

/// A stream with its own hidden sink and filter port group.
struct Slot {
    index: usize,
    app: u32,
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
}

#[derive(serde::Serialize)]
pub struct DeviceStatus {
    pub device: String,
    pub device_name: String,
    pub kind: Kind,
    pub profile: String,
    pub active: bool,
    pub present: bool,
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

    target: Option<u32>,
    out_links: Vec<Link>,
    /// Profile in effect and the (mode, tuning, rate) its engines were built for.
    profile: Option<Profile>,
    built: Option<(dts_eagle::Mode, dts_eagle::Output, u32)>,
    dirty_since: Option<Instant>,
    volume_quiet_until: Instant,
    /// Prevents a sink volume set before the device's is known from reaching the device.
    volume_synced: bool,
    ticks: u64,
}

pub type RouterRc = Rc<RefCell<Router>>;

fn prop<'a>(d: &'a DictRef, k: &str) -> Option<&'a str> {
    d.get(k)
}

impl Router {
    pub fn new(
        core: CoreRc,
        loop_: &pipewire::loop_::Loop,
        lib: &'static dts_eagle::Library,
        oem: Arc<dts_eagle::OemConfig>,
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
            target: None,
            out_links: Vec::new(),
            profile: None,
            built: None,
            dirty_since: None,
            volume_quiet_until: Instant::now(),
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
                        if let (Some(r), 0, Some(DEFAULT_KEY)) = (weak.upgrade(), subject, key) {
                            if let Ok(mut r) = r.try_borrow_mut() {
                                r.on_default_changed(value);
                            }
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
            let i = self.slots.iter().position(|s| s.sink_name == name).unwrap();
            self.move_app(i);
            self.try_link_all();
            return;
        }
        if class == "Stream/Output/Audio" {
            let Ok(proxy) = self.registry.bind::<Node, _>(g) else { return };
            let (id, weak) = (g.id, Rc::downgrade(this));
            let listener = proxy
                .add_listener_local()
                .param(move |_, pid, _, _, pod| {
                    if pid != ParamType::Format {
                        return;
                    }
                    let (Some(r), Some(pod)) = (weak.upgrade(), pod) else { return };
                    if let Some(pos) = parse_positions(pod) {
                        r.borrow_mut().on_app_format(id, pos);
                    }
                })
                .register();
            proxy.subscribe_params(&[ParamType::Format]);
            let serial = prop(props, "object.serial").and_then(|s| s.parse().ok()).unwrap_or(g.id);
            let dont_move = prop(props, "node.dont-move") == Some("true");
            self.apps.insert(
                g.id,
                App { serial, dont_move, positions: None, wanted: false, _proxy: proxy, _listener: listener },
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
            if let Some(i) = r.slots.iter().position(|s| s.app == id) {
                r.slots[i].moved = false; // the stream is gone, no target to clear
                r.release(i);
            }
        }
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

    fn on_app_format(&mut self, app: u32, positions: Vec<u32>) {
        let Some(a) = self.apps.get_mut(&app) else { return };
        let first = a.positions.is_none();
        a.positions = Some(positions);
        if first && a.wanted && !a.dont_move {
            self.add_slot(app);
        }
    }

    fn add_slot(&mut self, app: u32) {
        if self.slots.iter().any(|s| s.app == app) {
            return;
        }
        let Some(index) = (1..MAX_GROUPS).find(|i| !self.slots.iter().any(|s| s.index == *i)) else {
            log::warn!("too many streams; stream {app} shares the common input");
            return;
        };
        let a = &self.apps[&app];
        let channels = layout::group_layout(a.positions.as_deref().unwrap_or(&[]));
        let sink_name = format!("{IN_PREFIX}{}", a.serial);
        let sink: Node = match self.core.create_object(
            "adapter",
            &properties! {
                "factory.name" => "support.null-audio-sink",
                "node.name" => sink_name.as_str(),
                "node.description" => format!("AcerUltraX input {}", a.serial).as_str(),
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
                log::warn!("cannot create an input for stream {app}: {e}");
                return;
            }
        };
        let group = self.filter.add_group(index, a.serial, &channels);
        self.shared.groups.lock().unwrap().push(group);
        log::info!("stream {app} ({} ch) gets its own DTS engine", channels.len());
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
    }

    fn move_app(&mut self, i: usize) {
        let s = &mut self.slots[i];
        let Some((md, _)) = &self.metadata else { return };
        if s.moved || s.sink_id.is_none() {
            return;
        }
        md.set_property(s.app, "target.object", Some("Spa:Id"), Some(&s.sink_name));
        s.moved = true;
    }

    fn release(&mut self, i: usize) {
        let s = self.slots.remove(i);
        let group = {
            let mut groups = self.shared.groups.lock().unwrap();
            groups.iter().position(|g| g.index == s.index).map(|p| groups.remove(p))
        };
        if let Some(g) = group {
            self.filter.remove_group(g);
        }
        if s.moved && self.apps.contains_key(&s.app) {
            if let Some((md, _)) = &self.metadata {
                md.set_property(s.app, "target.object", None, None);
            }
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
    }

    /// Streams whose input could not be wired up within 3 s go back to the common input instead of silence.
    pub fn check_slots(&mut self) {
        while let Some(i) = self
            .slots
            .iter()
            .position(|s| s.created.elapsed() > Duration::from_secs(3) && s.links.iter().any(Option::is_none))
        {
            log::warn!("stream {}: its input is not connected after 3 s; using the common input", self.slots[i].app);
            if let Some(a) = self.apps.get_mut(&self.slots[i].app) {
                a.dont_move = true;
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
        if name == SINK_NAME || self.opts.no_default {
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
        self.check_slots();
        self.ticks += 1;
        if self.ticks % 300 == 0 {
            // Engines of idle apps are freed on the audio thread; hand that memory back to the system.
            unsafe { libc::malloc_trim(0) };
        }
        let rate = self.shared.rate.load(Ordering::Relaxed);
        if let Some((m, o, r)) = self.built {
            if rate != 0 && rate != r {
                self.build(m, o, rate);
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
        for ch in ["FL", "FR"] {
            let Some(o) = self.port(f, true, &format!("output_{ch}")) else { return };
            // Stereo devices get FL/FR; mono ones (e.g. Bluetooth headsets in call mode) get both on one port.
            let i = playback.iter().find(|(_, n)| n == &format!("playback_{ch}")).or(playback.first()).map(|p| p.0);
            if let Some(l) = i.and_then(|i| self.link(f, o, t, i, true)) {
                self.out_links.push(l);
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
            let tuning = kind.tuning();
            let rate = match self.shared.rate.load(Ordering::Relaxed) {
                0 => 48000,
                r => r,
            };
            if self.built.is_none_or(|(m, o, r)| (m, o, r) != (mode, tuning, rate)) {
                self.build(mode, tuning, rate);
            }
        }
        self.profile = Some(profile);
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

    /// Prepare a mode's engines on a worker thread; the audio thread picks them up.
    fn build(&mut self, mode: dts_eagle::Mode, tuning: dts_eagle::Output, rate: u32) {
        self.built = Some((mode, tuning, rate));
        if !dts_eagle::RATES.contains(&rate) {
            log::warn!("DTS cannot run at {rate} Hz; passing audio through");
            return;
        }
        let Some(t) = self.oem.tuning(mode, tuning).cloned() else { return };
        let (lib, shared) = (self.lib, self.shared.clone());
        let spawned = std::thread::Builder::new().name("dts-build".into()).stack_size(8 << 20).spawn(move || {
            match audio_graph::Prepared::build(lib, &t, rate) {
                Ok(p) => *shared.pending.lock().unwrap() = Some(p),
                Err(e) => log::error!("cannot prepare {mode} ({tuning:?}, {rate} Hz): {e}"),
            }
        });
        if let Err(e) = spawned {
            log::error!("cannot start a worker thread: {e}");
        }
        log::info!("mode {mode}, {tuning:?} tuning, {rate} Hz");
    }

    // ---------------------------------------------------------------- volume

    fn sync_volume_from_device(&mut self) {
        let (Some(t), Some(s)) = (self.target, self.sink_id) else { return };
        if let Some(v) = volume::get(t) {
            self.volume_quiet_until = Instant::now() + Duration::from_millis(500);
            volume::set(s, v);
            self.volume_synced = true;
        }
    }

    fn on_sink_props(&mut self, pod: &Pod) {
        let Some(v) = volume::parse_props(pod) else { return };
        if !self.volume_synced || Instant::now() < self.volume_quiet_until {
            return;
        }
        if let Some(t) = self.target {
            volume::set_async(t, v);
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
            streams: self.slots.len(),
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
                });
            }
        }
        rest.sort_by_key(|d| (!matches!(d.kind, Kind::Bluetooth | Kind::BluetoothSpeaker), d.device_name.clone()));
        v.extend(rest);
        v
    }

    /// Use `profile` on `device` (default: the current one; `@speakers`/`@headphones`: the laptop's own outputs).
    pub fn set_profile(&mut self, profile: &str, device: Option<&str>) -> Result<(), String> {
        let p = self.config.profile(profile).ok_or_else(|| format!("no profile named {profile:?}"))?;
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

//! The desktop application: an `eframe` window around the engine.
//!
//! The app is deliberately thin. It owns three things:
//!
//! 1. a [`Scenario`], which is the document;
//! 2. a simulated clock, which is a *display* clock — it decides which instant to show, and
//!    nothing else depends on it;
//! 3. a [`Camera`] and the input that moves it.
//!
//! Everything else is either the engine's or the viewport's. That is the rule from `AGENTS.md`
//! ("the UI is thin and swappable"): if a panel needed to compute something, the computation would
//! belong in the engine as a command, and the panel would run the command.
//!
//! # No `unwrap`
//!
//! The render loop runs thirty to sixty times a second, so it is the last place a panic is
//! acceptable. Every fallible step here — building a frame, propagating a satellite — has an
//! explicit branch, and the overlay *shows* what went wrong instead of logging it where nobody
//! looks.

use std::fmt::Write as _;
use std::time::Instant;

use eframe::egui;
use stkcraft_astro_engine::{Registry, Scenario};
use stkcraft_core::Angle;
use stkcraft_time::Epoch;
use stkcraft_ui_viewport::scene::SceneData;
use stkcraft_ui_viewport::{Camera, Frame, SceneCallback};

use crate::assistant::Assistant;
use crate::frame_builder::{build_frame, clamp_rate, step_duration};
use crate::prefs::{Prefs, system_locale};
use crate::render;

/// How many simulated seconds pass per second of wall time at the default rate.
pub const DEFAULT_RATE: f64 = 60.0;

/// How long an analysis is assumed to run when the app starts, seconds.
///
/// One day, so a low Earth orbit gets about fifteen passes: enough to be interesting and few
/// enough to fit in the panel.
pub const DEFAULT_ANALYSIS_SECONDS: f64 = 86_400.0;

/// The state the window draws from.
pub struct App {
    scenario: Scenario,
    camera: Camera,
    /// The instant currently displayed.
    now: Epoch,
    /// How many simulated seconds pass per wall second.
    rate: f64,
    /// Wall clock of the previous update, for the frame delta.
    last_update: Instant,
    /// The frame last built, so the overlay can report its contents.
    frame: Frame,
    /// Problems encountered building the last frame.
    problems: Vec<String>,
    /// The access table, recomputed only when the user asks.
    accesses: Vec<stkcraft_astro_engine::SatelliteAccess>,
    /// Status text for the panel: what happened last, and whether it worked.
    status: Option<(bool, String)>,
    /// The search step for the access table, seconds.
    access_step_seconds: f64,
    /// Whether to draw the graticule. A display toggle, not a command: nothing is computed.
    show_graticule: bool,
    /// The assistant panel: a model driving this scenario through the engine's commands.
    assistant: Assistant,
    /// The interface catalogs this build embedded.
    translations: stkcraft_i18n::Translations,
    /// The interface language, as a catalog code. English is `"en"`.
    language: String,
    /// What the window writes back when a preference changes.
    prefs: Prefs,
    /// A self-translation in flight: the person's own key filling in a language.
    translation: Option<crate::translate::Run>,
    /// What the language section says when a translation finished or could not start.
    translation_status: Option<(bool, String)>,
    /// The language the font stack was built for; a change to the language rebuilds it, because the
    /// Han faces are ordered by language.
    fonts_for: Option<String>,
}

impl App {
    /// Build the application around a scenario.
    #[must_use]
    pub fn new(scenario: Scenario, access_step_seconds: f64) -> Self {
        let now = scenario.epoch;
        let (frame, problems) = build_frame(&scenario, now);
        let prefs = Prefs::load();
        // Embedded catalogs first, then the ones this person generated: a catalog on disk is newer
        // than the one the binary was built with, and a language added without a rebuild works.
        let mut translations =
            stkcraft_i18n::Translations::from_embedded(crate::catalogs::EMBEDDED);
        for (locale, text) in crate::prefs::user_catalogs() {
            let (catalog, _problems) = stkcraft_i18n::parse(&locale, &text);
            translations.add(&catalog);
        }
        // The first run follows the system; an explicit choice always wins, because a person who
        // picked English on a German desktop meant it.
        let requested = prefs.language.clone().or_else(system_locale);
        let language = requested
            .as_deref()
            .map_or(stkcraft_i18n::Language::En, |tag| translations.resolve(tag))
            .code()
            .to_owned();
        Self {
            scenario,
            camera: Camera::default_view(),
            now,
            rate: DEFAULT_RATE,
            last_update: Instant::now(),
            frame,
            problems,
            accesses: Vec::new(),
            status: None,
            access_step_seconds,
            show_graticule: true,
            assistant: Assistant::default(),
            translations,
            language,
            prefs,
            translation: None,
            translation_status: None,
            fonts_for: None,
        }
    }

    /// Install the family font stack for the current language, if it is not already installed.
    ///
    /// Called from the window and from the offscreen verification path, so an image of a Chinese
    /// interface is drawn with the same faces the window uses. Without it `egui` has a Latin face
    /// only, and every translated label is tofu.
    pub fn install_fonts(&mut self, ctx: &egui::Context) {
        if self.fonts_for.as_deref() == Some(self.language.as_str()) {
            return;
        }
        crate::fonts::install(ctx, &self.language);
        self.fonts_for = Some(self.language.clone());
    }

    /// Rebuild the catalogs from what the build embeds and what this person has generated.
    ///
    /// Called after a self-translation writes one: the interface changes language in place rather
    /// than asking for a restart.
    fn reload_catalogs(&mut self) {
        let mut translations =
            stkcraft_i18n::Translations::from_embedded(crate::catalogs::EMBEDDED);
        for (locale, text) in crate::prefs::user_catalogs() {
            let (catalog, _problems) = stkcraft_i18n::parse(&locale, &text);
            translations.add(&catalog);
        }
        self.translations = translations;
    }

    /// Drain a self-translation, if one is running.
    fn poll_translation(&mut self) {
        let Some(run) = &mut self.translation else {
            return;
        };
        let mut finished = false;
        for event in run.poll() {
            match event {
                crate::translate::Event::Progress { locale, count } => {
                    self.translation_status = Some((
                        true,
                        format!("translating {count} string(s) into `{locale}`…"),
                    ));
                }
                crate::translate::Event::Done {
                    locale,
                    rows,
                    refused,
                } => {
                    self.reload_catalogs();
                    let mut message =
                        format!("`{locale}` is ready: {rows} string(s) translated with your key");
                    if refused > 0 {
                        let _ = write!(
                            message,
                            "; {refused} proposal(s) were refused by the format and still show \
                             English"
                        );
                    }
                    self.translation_status = Some((true, message));
                    finished = true;
                }
                crate::translate::Event::Failed(message) => {
                    self.translation_status = Some((false, message));
                    finished = true;
                }
            }
        }
        if finished {
            self.translation = None;
        }
    }

    /// Switch the interface language and remember the choice.
    ///
    /// The choice is written immediately rather than on exit: a window closed by a crash should not
    /// forget the language someone just picked, and one file write per click is nothing.
    fn set_language(&mut self, language: &stkcraft_i18n::Language) {
        language.code().clone_into(&mut self.language);
        // The font stack is language-dependent (Han glyph shapes), so the next frame rebuilds it.
        self.fonts_for = None;
        self.prefs.language = Some(self.language.clone());
        if let Err(error) = self.prefs.save() {
            eprintln!("stkcraft: {error}");
        }
    }

    /// Advance the simulated clock and rebuild the frame if it moved.
    ///
    /// Advancing by zero is a no-op rather than a rebuild: at a rate of zero the app is paused,
    /// and rebuilding the frame sixty times a second would burn a core for a still image.
    fn advance(&mut self, wall_seconds: f64) {
        let simulated = clamp_rate(self.rate) * wall_seconds;
        if simulated == 0.0 {
            return;
        }
        let offset = self.now.julian().plus_seconds(simulated);
        self.now = Epoch::from_julian(offset, self.now.scale());
        let (frame, problems) = build_frame(&self.scenario, self.now);
        self.frame = frame;
        self.problems = problems;
    }

    /// Rebuild the frame at the current instant, after something changed.
    fn refresh(&mut self) {
        let (frame, problems) = build_frame(&self.scenario, self.now);
        self.frame = frame;
        self.problems = problems;
    }

    /// Run the access analysis through the engine's command registry.
    ///
    /// A menu item or an agent would do exactly this, which is the point: the panel has no
    /// analysis of its own.
    fn compute_access(&mut self) {
        let registry = Registry::builtin();
        let step = step_duration(self.access_step_seconds);
        // The analysis is anchored at the scenario's own epoch, so the table does not move when
        // the display clock does.
        let mut analysis = self.scenario.clone();
        analysis.duration_seconds = DEFAULT_ANALYSIS_SECONDS;
        match registry.run(
            "access.compute",
            &mut analysis,
            &stkcraft_astro_engine::Args::none().with(
                "step-seconds",
                stkcraft_astro_engine::Arg::Number(step.as_seconds()),
            ),
        ) {
            Ok(outcome) => {
                self.accesses = outcome.access.unwrap_or_default();
                let passes: usize = self.accesses.iter().map(|a| a.windows.len()).sum();
                self.status = Some((
                    true,
                    format!(
                        "{passes} pass(es) over {} satellite/station pair(s).",
                        self.accesses.len()
                    ),
                ));
            }
            Err(error) => {
                self.accesses.clear();
                self.status = Some((false, format!("Access failed: {error}")));
            }
        }
    }

    /// The current frame's geometry, with the display toggles applied.
    ///
    /// One function for the window and for the verification renderer, so a screenshot cannot show
    /// a graticule the window hides.
    fn scene(&self) -> SceneData {
        let mut scene = SceneData::build(&self.frame, &self.camera);
        if !self.show_graticule {
            // The graticule is a display aid, not part of the frame, so dropping it here is
            // cheaper than teaching `SceneData` about display state.
            scene.graticule.clear();
        }
        scene
    }

    /// The current frame, scene and camera, for the verification renderer.
    ///
    /// Exposed rather than duplicated so a screenshot draws exactly what the window draws: if
    /// this disagreed with the window's own path, the image would be evidence of nothing.
    #[must_use]
    pub fn scene_for_verification(&self) -> (String, SceneData, Camera) {
        (self.frame.epoch_label.clone(), self.scene(), self.camera)
    }

    /// Move the displayed instant back to the scenario epoch.
    fn reset_clock(&mut self) {
        self.now = self.scenario.epoch;
        self.refresh();
    }

    /// Step the displayed instant by a number of seconds.
    fn step_clock(&mut self, seconds: f64) {
        let offset = self.now.julian().plus_seconds(seconds);
        self.now = Epoch::from_julian(offset, self.now.scale());
        self.refresh();
    }

    /// Open the assistant panel, for a caller that wants it visible from the start.
    ///
    /// Only the verification path uses this: the window opens with the panel closed, because a
    /// panel that starts open is a panel that starts asking a model for a key.
    pub fn open_assistant(&mut self) {
        self.assistant.open = true;
    }

    /// The panels, without the viewport: the window draws these and then the scene, and the
    /// verification path draws these alone so a panel can be looked at on a machine with no window.
    ///
    /// Kept as one function for the same reason the scene is: an image that laid the panels out
    /// itself would be evidence of a layout the window does not have.
    pub fn panels(&mut self, ui: &mut egui::Ui) {
        self.install_fonts(ui.ctx());
        self.poll_translation();
        if self.translation.is_some() {
            // A translation is a network round trip; keep drawing so its result is picked up.
            ui.ctx().request_repaint();
        }
        egui::Panel::left("controls")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| self.left_panels(ui));

        if self.assistant.open {
            let skills = crate::cli::skills_directory();
            let mut adopted = None;
            let language = self.language.clone();
            let translations = self.translations.clone();
            egui::Panel::right("assistant")
                .resizable(true)
                .default_size(360.0)
                .show(ui, |ui| {
                    adopted =
                        self.assistant
                            .ui(ui, &self.scenario, &skills, &translations, &language);
                });
            if let Some(scenario) = adopted {
                self.scenario = scenario;
                self.refresh();
            }
        }
        // While a question runs there is a worker to collect from, so keep drawing even when the
        // clock is paused; otherwise the answer would sit in the channel until the next input.
        if self.assistant.is_running() {
            ui.ctx().request_repaint();
        }
    }

    /// The `egui` side panel: clock, scenario contents, parameters and results.
    fn left_panels(&mut self, ui: &mut egui::Ui) {
        // The language is copied out so a lookups and the widget borrows below cannot fight over
        // `self`; every literal goes through `tr!` so `cargo xtask i18n` can find it.
        let language = self.language.clone();
        macro_rules! tr {
            ($source:literal) => {
                stkcraft_i18n::tr!(&self.translations, &language, $source)
            };
        }
        ui.heading("STKCraft");
        ui.label(format!("{} {}", tr!("version"), env!("CARGO_PKG_VERSION")));
        ui.separator();

        ui.heading(tr!("Clock"));
        ui.label(monospace(&self.frame.epoch_label));
        ui.horizontal(|ui| {
            if ui.button(tr!("−1 h")).clicked() {
                self.step_clock(-3_600.0);
            }
            if ui.button(tr!("Reset")).clicked() {
                self.reset_clock();
            }
            if ui.button(tr!("+1 h")).clicked() {
                self.step_clock(3_600.0);
            }
        });
        // A slider that includes zero and negatives, with a logarithmic feel around 1x.
        ui.add(
            egui::Slider::new(&mut self.rate, -600.0..=600.0)
                .text(tr!("time rate"))
                .suffix("x"),
        );
        if ui.button(tr!("Pause")).clicked() {
            self.rate = 0.0;
        }

        ui.separator();
        ui.heading(tr!("Scenario"));
        let satellites = self.scenario.satellites.len() as u64;
        ui.label(stkcraft_i18n::trn!(
            &self.translations,
            &language,
            "{n} satellite",
            "{n} satellites",
            satellites
        ));
        for satellite in &self.scenario.satellites {
            let [r, g, b] = satellite.color;
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(r, g, b), "\u{25CF}");
                ui.label(&satellite.name);
            });
        }
        let stations = self.scenario.stations.len() as u64;
        ui.label(stkcraft_i18n::trn!(
            &self.translations,
            &language,
            "{n} station",
            "{n} stations",
            stations
        ));
        for station in &self.scenario.stations {
            ui.label(format!(
                "  {}  {:.3}, {:.3}",
                station.name,
                station.site.latitude.as_deg(),
                station.site.longitude.as_deg()
            ));
        }

        ui.separator();
        ui.heading(tr!("Access"));
        ui.add(
            egui::Slider::new(&mut self.access_step_seconds, 5.0..=120.0)
                .text(tr!("search step"))
                .suffix(" s"),
        );
        if ui.button(tr!("Compute passes")).clicked() {
            self.compute_access();
        }
        if let Some((ok, message)) = &self.status {
            let colour = if *ok {
                egui::Color32::from_rgb(140, 200, 140)
            } else {
                egui::Color32::from_rgb(230, 120, 120)
            };
            ui.colored_label(colour, message);
        }
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .show(ui, |ui| {
                for access in &self.accesses {
                    let satellite = self
                        .scenario
                        .satellite(access.satellite)
                        .map_or("?", |s| s.name.as_str());
                    let station = self
                        .scenario
                        .station(access.station)
                        .map_or("?", |s| s.name.as_str());
                    ui.label(format!(
                        "{satellite} over {station}: {} pass(es), {}",
                        access.windows.len(),
                        render::format_duration(access.total.as_seconds())
                    ));
                    for window in access.windows.iter().take(20) {
                        let start = instant_at(self.scenario.epoch, window.start.as_seconds());
                        ui.label(format!(
                            "   {}  {}",
                            render::format_epoch(start),
                            render::format_duration(window.duration().as_seconds())
                        ));
                    }
                }
            });

        ui.separator();
        ui.heading(tr!("View"));
        ui.checkbox(&mut self.show_graticule, tr!("Graticule"));
        ui.checkbox(&mut self.assistant.open, tr!("Assistant panel"));
        if ui.button(tr!("Reset camera")).clicked() {
            self.camera = Camera::default_view();
        }
        let address = format!("{:.3}, {:.3}", self.camera.azimuth, self.camera.elevation);
        ui.label(format!("{} {address}", tr!("azimuth, elevation:")));
        ui.label(format!(
            "{} {:.0} km",
            tr!("distance:"),
            self.camera.distance_km
        ));

        ui.separator();
        ui.heading(tr!("Language"));
        let current =
            stkcraft_i18n::Language::parse(&language).unwrap_or(stkcraft_i18n::Language::En);
        let mut chosen = current.clone();
        egui::ComboBox::from_id_salt("language-picker")
            .selected_text(self.translations.language_name(current.code()))
            .show_ui(ui, |ui| {
                // First the languages this build carries: English and every shipped catalog.
                for language in self.translations.picker() {
                    let name = self.translations.language_name(language.code());
                    ui.selectable_value(&mut chosen, language.clone(), name);
                }
                ui.separator();
                // Then the ones it does not, which the person can generate with their own key.
                for code in crate::prefs::OFFERED {
                    if self.translations.has_locale(code) {
                        continue;
                    }
                    let language = stkcraft_i18n::Language::Other(code.to_owned());
                    ui.selectable_value(
                        &mut chosen,
                        language,
                        crate::prefs::language_name_for(code),
                    );
                }
            });
        if chosen != current {
            self.set_language(&chosen);
        }

        // Self-translation: offered only when there is something to translate and English is not the
        // answer someone wants.
        if !stkcraft_i18n::Language::parse(&language).is_some_and(|language| language.is_english())
        {
            let missing = crate::translate::missing(&self.translations, &language);
            if self.translation.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tr!("Translating with your key…"));
                });
            } else if missing > 0 {
                let label = stkcraft_i18n::trn!(
                    &self.translations,
                    &language,
                    "Translate {n} string with my key",
                    "Translate {n} strings with my key",
                    missing as u64
                );
                if ui.button(label).on_hover_text(tr!(
                    "Uses DEEPSEEK_API_KEY to translate the interface into this language and stores \
                     it for this computer."
                ))
                .clicked()
                {
                    match crate::translate::start(&language) {
                        Ok(run) => {
                            self.translation = Some(run);
                            self.translation_status = None;
                        }
                        Err(message) => self.translation_status = Some((false, message)),
                    }
                }
            }
        }
        if let Some((ok, message)) = &self.translation_status {
            let colour = if *ok {
                egui::Color32::from_rgb(140, 200, 140)
            } else {
                egui::Color32::from_rgb(230, 120, 120)
            };
            ui.colored_label(colour, message);
        }

        ui.separator();
        ui.heading(tr!("Diagnostics"));
        ui.label(format!(
            "{} satellite marker(s), {} station(s), {} track(s)",
            self.frame.satellites.len(),
            self.frame.stations.len(),
            self.frame.tracks.len()
        ));
        for problem in &self.problems {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 120), problem);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // Before anything is drawn: a label in a script no installed face covers is tofu.
        self.install_fonts(ui.ctx());
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        self.last_update = now;
        // A hitch must not teleport the clock: a frame that took a second to draw should not
        // advance a minute of simulated time. Clamped to a quarter second.
        self.advance(elapsed.min(0.25));

        self.panels(ui);

        // No panel frame: the viewport is the whole area, and a theme-coloured margin around it
        // would look like a letterbox rather than space.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let (response, painter) =
                    ui.allocate_painter(ui.available_size(), egui::Sense::drag());
                let rect = response.rect;

                // Camera input: drag orbits, wheel zooms.
                if response.dragged() {
                    let delta = response.drag_delta();
                    let scale = 0.005;
                    self.camera.orbit(-delta.x * scale, delta.y * scale);
                }
                if response.hovered() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0.0 {
                        // An exponential zoom, so each notch changes the distance by a constant
                        // ratio rather than a constant number of kilometres.
                        self.camera.zoom((-scroll * 0.0015).exp());
                    }
                }

                // The background: space, not the theme's panel colour.
                painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(4, 6, 12));

                // The scene itself, through the same `wgpu` pipelines the offscreen verification path
                // uses. The engine's frame is turned into geometry once, here, and both paths consume
                // the same `SceneData`, so a screenshot cannot agree with a window that is drawing
                // something else.
                //
                // Without a render state — a backend other than `wgpu` — there is nothing to draw the
                // scene with; the overlay still shows the frame's contents, so the window is legible
                // rather than empty.
                if let Some(render_state) = frame.wgpu_render_state() {
                    let scene = self.scene();
                    let aspect = rect.width() / rect.height();
                    painter.add(eframe::egui_wgpu::Callback::new_paint_callback(
                        rect,
                        SceneCallback::new(scene, &self.camera, aspect, render_state.target_format),
                    ));
                }

                // The overlay: the clock, floating over the view.
                painter.text(
                    rect.left_top() + egui::vec2(12.0, 10.0),
                    egui::Align2::LEFT_TOP,
                    &self.frame.epoch_label,
                    egui::FontId::monospace(15.0),
                    egui::Color32::from_rgb(200, 210, 230),
                );
            });

        // Keep the clock running. Without this an idle window would stop advancing the timeline.
        if self.rate != 0.0 {
            ui.ctx().request_repaint();
        }
    }
}

/// An absolute instant from a scenario epoch and an offset.
fn instant_at(epoch: Epoch, offset_seconds: f64) -> Epoch {
    Epoch::from_julian(epoch.julian().plus_seconds(offset_seconds), epoch.scale())
}

/// Monospaced text, so a clock does not jitter as digits change.
fn monospace(text: &str) -> egui::RichText {
    egui::RichText::new(text).monospace()
}

/// Run the window.
///
/// # Errors
///
/// Returns the message `eframe` gave when the window could not be created, which on a headless
/// machine is the expected outcome and must be reported rather than swallowed.
pub fn run(options: eframe::NativeOptions, app: App) -> Result<(), String> {
    eframe::run_native(
        "STKCraft",
        options,
        Box::new(move |_creation| Ok(Box::new(app))),
    )
    .map_err(|error| error.to_string())
}

/// The demo scenario the app opens with when no file is given.
///
/// One ISS element set and one station, both from public data: the element set is the one in
/// `stkcraft-astro-sgp4`'s verification fixtures and the station is London's well-known coordinate.
/// Chosen so that the very first frame is computed rather than empty.
///
/// # Errors
///
/// Returns the parse error if either built-in element set is malformed, which would be a bug in
/// this crate rather than a user error, but is reported rather than unwrapped.
pub fn demo_scenario() -> Result<Scenario, String> {
    use stkcraft_astro_access::{Ellipsoid, GroundSite, Sensor};
    use stkcraft_astro_sgp4::TwoLineElement;

    let iss = TwoLineElement::parse(
        "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9009",
        "2 25544  51.6400 208.9163 0006317  69.9862  25.2906 15.49569600 12343",
    )
    .map_err(|error| format!("the built-in ISS element set is invalid: {error}"))?;

    let mut scenario = Scenario::empty("Demo");
    // Start at the element set's own epoch. A scenario starting "now" would be years from these
    // elements and every propagation would be nonsense.
    scenario.epoch = iss.epoch;
    scenario.duration_seconds = DEFAULT_ANALYSIS_SECONDS;
    scenario
        .add_satellite("ISS", iss, [240, 90, 60])
        .map_err(|error| format!("could not add the ISS: {error}"))?;

    let london = GroundSite::new(Angle::from_deg(51.5), Angle::from_deg(-0.13), 0.0)
        .map_err(|error| format!("London is not a valid site: {error}"))?;
    scenario
        .add_station(
            "London",
            london,
            Sensor::above_horizon(Angle::from_deg(10.0)),
        )
        .map_err(|error| format!("could not add the station: {error}"))?;
    // The `Ellipsoid` is used implicitly by the access problem and by the frame builder; naming it
    // here documents that the demo and the engine share WGS-84.
    assert_eq!(Ellipsoid::WGS84.semi_major_km, 6_378.137);
    Ok(scenario)
}

/// The `eframe` options the app runs with.
#[must_use]
pub fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0])
            .with_title("STKCraft"),
        renderer: eframe::Renderer::Wgpu,
        // The viewport draws a depth-tested globe, so the window's pass needs a depth attachment.
        // `eframe` takes bits and `wgpu` takes a format; the constant ties the two together and a
        // test in the viewport crate pins the mapping.
        depth_buffer: stkcraft_ui_viewport::renderer::WINDOW_DEPTH_BITS,
        ..eframe::NativeOptions::default()
    }
}

#[cfg(test)]
#[expect(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_scenario_builds_and_propagates() {
        // The first frame must not be empty: an empty window looks like a crash.
        let scenario = demo_scenario().expect("the demo scenario is valid");
        assert_eq!(scenario.satellites.len(), 1);
        assert_eq!(scenario.stations.len(), 1);
        let (frame, problems) = build_frame(&scenario, scenario.epoch);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(frame.satellites.len(), 1);
        assert_eq!(frame.stations.len(), 1);
        assert_ne!(frame.tracks, [] as [std::vec::Vec<[f32; 3]>; 0]);
    }

    #[test]
    fn the_clock_advances_by_the_rate_and_stops_at_zero() {
        let scenario = demo_scenario().expect("valid");
        let mut app = App::new(scenario, 30.0);
        let before = app.now.julian().sum();
        app.advance(1.0);
        let after = app.now.julian().sum();
        // 60 simulated seconds per wall second, in days.
        let expected = DEFAULT_RATE / 86_400.0;
        assert!(
            ((after - before) - expected).abs() < 1e-9,
            "the clock advanced {} days, expected {expected}",
            after - before
        );

        app.rate = 0.0;
        let paused = app.now.julian().sum();
        app.advance(10.0);
        assert_eq!(app.now.julian().sum(), paused, "a paused clock moved");
    }

    #[test]
    fn a_nonsense_rate_is_ignored_rather_than_breaking_the_clock() {
        let scenario = demo_scenario().expect("valid");
        let mut app = App::new(scenario, 30.0);
        app.rate = f64::NAN;
        app.advance(1.0);
        // `clamp_rate` substitutes 1x, so the clock moved by one second, not by NaN.
        assert!(app.now.julian().is_finite());
    }

    #[test]
    fn computing_access_fills_the_table_through_the_registry() {
        // The panel must go through the engine's command, not a private implementation.
        let scenario = demo_scenario().expect("valid");
        let mut app = App::new(scenario, 30.0);
        app.compute_access();
        let (ok, message) = app.status.clone().expect("a status was set");
        assert!(ok, "access failed: {message}");
        assert_eq!(app.accesses.len(), 1);
        assert!(
            app.accesses.first().is_some_and(|a| !a.windows.is_empty()),
            "the ISS never passed over London in 24 hours"
        );
    }

    #[test]
    fn the_graticule_toggle_only_affects_the_graticule() {
        // The window used to draw geometry itself; it now hands `SceneData` to the GPU, and this
        // pins that the display toggle survived the move — a toggle that did nothing would be a
        // silent regression.
        let scenario = demo_scenario().expect("valid");
        let app = App::new(scenario, 30.0);
        let (_, with_graticule, _) = app.scene_for_verification();
        assert_ne!(
            with_graticule.graticule,
            [] as [stkcraft_ui_viewport::Vertex; 0]
        );

        let mut without = App::new(app.scenario.clone(), 30.0);
        without.show_graticule = false;
        let (_, scene, _) = without.scene_for_verification();
        assert!(
            scene.graticule.is_empty(),
            "the graticule was drawn with the toggle off"
        );
    }

    #[test]
    fn the_embedded_catalogs_are_well_formed_and_translate() {
        // `cargo xtask i18n` is the gate; this is the smoke test that the catalogs a build embeds
        // actually answer. A catalog that parses in the task but not here would be a build-input
        // problem, which is exactly the failure mode of an embedded file.
        let translations = stkcraft_i18n::Translations::from_embedded(crate::catalogs::EMBEDDED);
        for locale in translations.locales() {
            assert!(
                translations.len(locale) > 0,
                "{locale} embeds an empty catalog"
            );
        }
        if translations.has_locale("zh") {
            let clock = translations.tr("zh", "Clock");
            assert_ne!(
                clock, "Clock",
                "the Chinese catalog does not translate `Clock`"
            );
            assert!(!clock.is_ascii(), "`{clock}` is not Chinese");
            // The chain has to reach it from what an operating system reports.
            assert_eq!(
                translations.resolve("zh_CN.UTF-8").code(),
                "zh",
                "a Chinese system locale does not resolve to the Chinese catalog"
            );
        }
    }
}

use super::{Mode, NetworkMapApp};
use eframe::egui::{self, RichText};
use global_network_map::{
    config::{Config, Origin},
    geo::{self, City},
    network,
};
use std::sync::mpsc;

pub struct Draft {
    pub origin: Origin,
    pub interface: Option<String>,
    query: String,
    cities: Vec<City>,
    matches: Vec<usize>,
    pub error: String,
    suggestion: Option<Origin>,
    suggestion_ip: String,
}
impl Draft {
    pub fn new(config: &Config) -> Self {
        Self {
            origin: config.origin.clone().unwrap_or(Origin {
                city: String::new(),
                country: String::new(),
                latitude: 0.,
                longitude: 0.,
            }),
            interface: config.interface.clone(),
            query: String::new(),
            cities: vec![],
            matches: vec![],
            error: String::new(),
            suggestion: None,
            suggestion_ip: String::new(),
        }
    }
}
pub enum JobResult {
    Database(Result<(), String>),
    Suggestion(Result<(String, Option<Origin>), String>),
}
impl NetworkMapApp {
    pub(super) fn poll_job(&mut self) {
        let result = self.job.as_ref().and_then(|r| r.try_recv().ok());
        if let Some(result) = result {
            self.job = None;
            match result {
                JobResult::Database(Ok(())) => {
                    self.job_status =
                        "IPv4 and IPv6 databases updated. Start a new capture session to use them."
                            .into();
                    if self.capture.is_none() {
                        match geo::GeoDb::open(
                            self.args
                                .database
                                .as_deref()
                                .or(self.config.database.as_deref()),
                        ) {
                            Ok(db) => {
                                self.db = std::sync::Arc::new(db);
                                self.job_status = "City databases updated and ready.".into();
                            }
                            Err(e) => {
                                self.job_status = format!(
                                "Downloaded databases, but could not reopen location data: {e:#}"
                            )
                            }
                        }
                    }
                }
                JobResult::Database(Err(e)) => {
                    self.job_status = format!("Database update failed: {e}")
                }
                JobResult::Suggestion(Ok((ip, origin))) => {
                    self.draft.suggestion = origin;
                    self.draft.suggestion_ip = ip;
                    self.job_status=if self.draft.suggestion.is_some(){"Exit-IP suggestion is ready. It has not changed your configured origin."}else{"No city found for the exit IP. Download the database or select your city manually."}.into();
                }
                JobResult::Suggestion(Err(e)) => {
                    self.job_status = format!("Public-IP suggestion failed: {e}")
                }
            }
        }
    }
    pub(super) fn settings(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        let mut saved = false;
        egui::Window::new("Settings").open(&mut open).collapsible(false).resizable(false).default_width(590.).show(ctx,|ui| {
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height()-140.).show(ui,|ui| {
                ui.heading("Your map origin");
                ui.label("Choose your physical city. This stays independent of your VPN exit.");
                ui.add_space(4.);
                let search=ui.add(egui::TextEdit::singleline(&mut self.draft.query).hint_text("Search city and country, e.g. Paris, FR").desired_width(f32::INFINITY));
                if search.changed() {
                    if self.draft.cities.is_empty(){self.draft.cities=geo::cities();}
                    self.draft.matches=geo::search_cities(&self.draft.cities,&self.draft.query);
                }
                if !self.draft.query.is_empty() {
                    egui::ScrollArea::vertical().id_salt("city-results").max_height(135.).show(ui,|ui| {
                        if self.draft.matches.is_empty(){ui.label("No match. Enter your city and coordinates below.");}
                        for &index in &self.draft.matches {
                            let city=&self.draft.cities[index];
                            if ui.selectable_label(self.draft.origin.city==city.origin.city&&self.draft.origin.country==city.origin.country,
                                format!("{} · {} ({}, {})",city.origin.city,city.origin.country,city.code,city.region)).clicked() {
                                self.draft.origin=city.origin.clone();
                            }
                        }
                    });
                }
                ui.add_space(4.);
                egui::Grid::new("origin-fields").num_columns(2).spacing([16.,8.]).show(ui,|ui| {
                    ui.label("City");ui.text_edit_singleline(&mut self.draft.origin.city);ui.end_row();
                    ui.label("Country");ui.text_edit_singleline(&mut self.draft.origin.country);ui.end_row();
                    ui.label("Latitude");ui.add(egui::DragValue::new(&mut self.draft.origin.latitude).speed(0.01).range(-90.0..=90.0).max_decimals(5));ui.end_row();
                    ui.label("Longitude");ui.add(egui::DragValue::new(&mut self.draft.origin.longitude).speed(0.01).range(-180.0..=180.0).max_decimals(5));ui.end_row();
                });
                ui.add_space(6.);
                if ui.add_enabled(self.job.is_none(),egui::Button::new("Suggest from public exit IP…")).clicked() {
                    let db=self.db.clone();let (sender,receiver)=mpsc::channel();self.job=Some(receiver);
                    self.job_status="Looking up the public exit IP…".into();
                    std::thread::spawn(move||{
                        let result=network::public_location(&db).map(|(ip,l)|{
                            let origin=l.coordinates().map(|(lon,lat)|Origin{city:l.city,country:l.country,latitude:lat,longitude:lon});
                            (ip.to_string(),origin)
                        }).map_err(|e|format!("{e:#}"));
                        let _=sender.send(JobResult::Suggestion(result));
                    });
                }
                ui.small("Contacts api.ipify.org only when clicked. With a VPN this suggests its exit city, not your physical location.");
                if let Some(origin)=&self.draft.suggestion {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}, {} ({})",origin.city,origin.country,self.draft.suggestion_ip));
                        if ui.small_button("Use suggestion").clicked(){self.draft.origin=origin.clone();}
                    });
                }
                ui.add_space(8.);ui.separator();ui.heading("Capture interface");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("interface").width(370.).selected_text(self.draft.interface.as_deref().unwrap_or("Automatic · current Internet route")).show_ui(ui,|ui| {
                        ui.selectable_value(&mut self.draft.interface,None,"Automatic · current Internet route");
                        for device in &self.interfaces {
                            ui.selectable_value(&mut self.draft.interface,Some(device.name.clone()),format!("{}{} · {}",device.name,if device.tunnel{" (VPN/tunnel)"}else{""},device.addresses));
                        }
                    });
                    if ui.small_button("Refresh").clicked(){match network::interfaces(){Ok(list)=>self.interfaces=list,Err(e)=>self.draft.error=e.to_string()};}
                });
                ui.small("Use the VPN tunnel to see inner destinations. One interface avoids double-counting. Restart capture after changing your VPN; split-tunnel traffic on other interfaces is excluded.");
                if self.args.interface.is_some(){ui.colored_label(self.palette.accent,"A command-line interface override is active until this app is restarted.");}
                ui.add_space(8.);ui.separator();ui.heading("Offline location data");
                ui.label(&self.db.status);
                if ui.add_enabled(self.job.is_none(),egui::Button::new("Download / update city databases")).clicked() {
                    let (sender,receiver)=mpsc::channel();self.job=Some(receiver);self.job_status="Downloading and verifying IPv4 + IPv6 data…".into();
                    std::thread::spawn(move||{let result=network::update_db().map_err(|e|format!("{e:#}"));let _=sender.send(JobResult::Database(result));});
                }
                ui.small("Downloads DB-IP Lite from GitHub. Normal monitoring uses local data only. IP geolocation is approximate.");
                if self.job.is_some(){ui.horizontal(|ui|{ui.spinner();ui.label(&self.job_status);});}
                else if !self.job_status.is_empty(){ui.label(&self.job_status);}
                ui.add_space(8.);ui.separator();ui.label("Appearance follows your active Omarchy theme automatically. Inbound uses theme red; outbound uses theme green.");
                if !self.draft.error.is_empty(){ui.colored_label(self.palette.incoming,&self.draft.error);}
                ui.add_space(8.);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Save settings").strong().color(self.palette.outgoing)).clicked() {
                        let result=(||->anyhow::Result<()> {
                            self.draft.origin.validate()?;
                            let mut config=self.config.clone();config.origin=Some(self.draft.origin.clone());config.interface=self.draft.interface.clone();config.save()?;
                            if config.interface!=self.config.interface && matches!(self.mode,Mode::Live|Mode::Authorizing){self.stop();}
                            self.config=config;Ok(())
                        })();
                        match result {Ok(())=>{saved=true;self.draft.error.clear();},Err(e)=>self.draft.error=format!("{e:#}")}
                    }
                    ui.small("Changing the interface stops capture; Start begins a new session.");
                });
            });
        });
        self.settings_open = open && !saved;
    }
}

use anyhow::Result;
use omni_engine::{AudioEngine, EngineCommand};
use crossbeam_channel::{unbounded, Sender, Receiver};
use eframe::egui;

pub struct OmniApp {
    is_playing: bool,
    volume: f32,
    messenger: Sender<EngineCommand>,
    receiver: Option<Receiver<EngineCommand>>,
    engine: Option<AudioEngine>,
}

impl OmniApp {
    fn new(tx: Sender<EngineCommand>, rx: Receiver<EngineCommand>) -> Self {
        Self {
            is_playing: false,
            volume: 0.1,
            messenger: tx,
            receiver: Some(rx),
            engine: None,
        }
    }
}

impl eframe::App for OmniApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Omni DAW");
            ui.add_space(20.0);

            let label = if self.is_playing { "STOP" } else { "PLAY" };
            if ui.button(label).clicked() {
                // Lazy Engine Init
                if self.engine.is_none() {
                    if let Some(rx) = self.receiver.take() {
                        if let Ok(e) = AudioEngine::new(rx) {
                            self.engine = Some(e);
                        }
                    }
                }

                self.is_playing = !self.is_playing;
                let cmd = if self.is_playing { EngineCommand::Play } else { EngineCommand::Stop };
                let _ = self.messenger.send(cmd);
            }

            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.label("Volume:");
                if ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.0)).changed() {
                    let _ = self.messenger.send(EngineCommand::SetVolume(self.volume));
                }
            });

            if self.engine.is_none() && self.is_playing {
                ui.colored_label(egui::Color32::RED, "Engine failed to initialize");
            }
        });
    }
}

fn main() -> Result<()> {
    let (tx, rx) = unbounded();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([300.0, 200.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Omni",
        options,
        Box::new(|_cc| {
            Ok(Box::new(OmniApp::new(tx, rx)))
        }),
    ).map_err(|e| anyhow::anyhow!("Eframe error: {}", e))?;

    Ok(())
}

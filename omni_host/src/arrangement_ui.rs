use eframe::egui;
use omni_engine::EngineCommand;
use crossbeam_channel::Sender;
use std::collections::HashMap;

/// Cached waveform peaks for an asset at a specific resolution
#[derive(Clone)]
pub struct WaveformCache {
    pub _asset_id: u32,
    pub samples_per_peak: usize,
    pub peaks: Vec<(f32, f32)>, // (min, max) per pixel/column
    pub width: f32, // Cache invalidation if width changes drastically? No, samples_per_peak covers it.
}

pub struct ArrangementUI {
    // Zoom / Scroll State
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub zoom_x: f32, // Pixels per beat (Default 50.0)
    pub zoom_y: f32, // Pixels per track height (Default 60.0)
    pub header_width: f32, // Width of track headers
    
    // Interaction State
    pub drag_state: Option<DragState>,
    
    // Waveform Cache: asset_id -> cached peaks
    waveform_cache: HashMap<u32, WaveformCache>,
}

#[derive(Clone, Copy, Debug)]
pub struct DragState {
    pub track_index: usize,
    pub clip_index: usize,
    pub original_start_samples: u64,
    pub start_mouse_x: f32,
}

impl Default for ArrangementUI {
    fn default() -> Self {
        Self {
            scroll_x: 0.0,
            scroll_y: 0.0,
            zoom_x: 50.0, 
            zoom_y: 60.0, 
            header_width: 150.0,
            drag_state: None,
            waveform_cache: HashMap::new(),
        }
    }
}

impl ArrangementUI {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        tracks: &mut Vec<crate::TrackData>,
        bpm: f32,
        sender: &Sender<EngineCommand>,
        _current_step: u32, 
        playback_pos_samples: u64,
        sample_rate: f32,
        audio_pool: Option<&std::sync::Arc<arc_swap::ArcSwap<omni_engine::assets::AudioPool>>>,
    ) {
        let max_rect = ui.available_rect_before_wrap();
        // ui.set_clip_rect(max_rect); // Clip to available space

        // 1. Handle Scrolling Input (Pan with Middle Mouse or Alt+Drag)
        let response = ui.interact(max_rect, ui.id().with("arr_bg"), egui::Sense::click_and_drag());
        if response.dragged_by(egui::PointerButton::Middle) || (ui.input(|i| i.modifiers.alt) && response.dragged()) {
            self.scroll_x -= response.drag_delta().x;
            self.scroll_y -= response.drag_delta().y;
        }

        // Clamp Scroll
        self.scroll_x = self.scroll_x.max(0.0);
        self.scroll_y = self.scroll_y.max(0.0);
        
        // 2. Layout Dimensions
        let top_ruler_height = 30.0;
        let content_rect = egui::Rect::from_min_max(
            max_rect.min + egui::vec2(0.0, top_ruler_height),
            max_rect.max
        );
        let ruler_rect = egui::Rect::from_min_max(
            max_rect.min,
            egui::pos2(max_rect.max.x, max_rect.min.y + top_ruler_height)
        );

        let painter = ui.painter_at(max_rect);

        // --- DRAW RULER ---
        // Background
        painter.rect_filled(ruler_rect, 0.0, crate::ui::theme::THEME.bg_medium);
        
        let visible_start_beat = (self.scroll_x / self.zoom_x).floor() as i64;
        let visible_width_beats = (max_rect.width() / self.zoom_x).ceil() as i64 + 1;
        
        for b in 0..visible_width_beats {
            let beat = visible_start_beat + b;
            if beat < 0 { continue; }
            
            let x = (beat as f32 * self.zoom_x) - self.scroll_x + self.header_width + max_rect.min.x;
            
            if x >= max_rect.min.x + self.header_width {
                // Major Tick (Bar/Beat)
                // Assuming 4/4 for now
                let is_bar = beat % 4 == 0;
                let color = if is_bar { crate::ui::theme::THEME.text_primary } else { crate::ui::theme::THEME.text_secondary };
                let height = if is_bar { 15.0 } else { 8.0 };
                
                painter.line_segment(
                    [egui::pos2(x, ruler_rect.bottom()), egui::pos2(x, ruler_rect.bottom() - height)],
                    (1.0, color)
                );
                
                if is_bar {
                    let text = format!("{}", (beat / 4) + 1);
                    painter.text(
                        egui::pos2(x + 5.0, ruler_rect.bottom() - 20.0),
                        egui::Align2::LEFT_CENTER,
                        text,
                        egui::FontId::proportional(12.0),
                        crate::ui::theme::THEME.text_primary,
                    );
                }
            }
        }
        
        // --- DRAW TRACKS ---
        let _visible_start_track = (self.scroll_y / self.zoom_y).floor() as usize;
        let _visible_count = (content_rect.height() / self.zoom_y).ceil() as usize + 1;
        
        // Headers Background
        let headers_rect = egui::Rect::from_min_size(
             content_rect.min,
             egui::vec2(self.header_width, content_rect.height())
        );
        painter.rect_filled(headers_rect, 0.0, crate::ui::theme::THEME.bg_medium);
        
        // Draw Grid Background lines
        let grid_rect = egui::Rect::from_min_max(
            egui::pos2(content_rect.min.x + self.header_width, content_rect.min.y),
            content_rect.max
        );
        
        // Clip Rect for Grid
        ui.set_clip_rect(grid_rect);

        for i in 0..tracks.len() {
             let screen_y = content_rect.min.y + (i as f32 * self.zoom_y) - self.scroll_y;
             
             // Check visibility
             if screen_y + self.zoom_y < content_rect.min.y || screen_y > content_rect.max.y {
                 continue;
             }
             
             // Draw Header
             let header_item_rect = egui::Rect::from_min_size(
                 egui::pos2(content_rect.min.x, screen_y),
                 egui::vec2(self.header_width, self.zoom_y)
             );
             
             // Need to temporarily unclip for header? Or paint header over grid?
             // Painter order matters. We are painting to same layer.
             // We logic-ed clipping via `ui.set_clip_rect` which affects widgets added via `ui.add`.
             // But `painter` ignores it unless we create a sub-painter with clip.
             // Let's just draw.
             
             painter.rect_filled(header_item_rect, 0.0, crate::ui::theme::THEME.bg_light);
             painter.rect_stroke(header_item_rect, 0.0, (1.0, crate::ui::theme::THEME.border), egui::StrokeKind::Middle);
             
             painter.text(
                 header_item_rect.left_center() + egui::vec2(10.0, 0.0),
                 egui::Align2::LEFT_CENTER,
                 &tracks[i].name,
                 egui::FontId::proportional(14.0),
                 crate::ui::theme::THEME.text_primary
             );
             
             // Draw Grid Row Background
             let row_rect = egui::Rect::from_min_max(
                 egui::pos2(grid_rect.min.x, screen_y),
                 egui::pos2(grid_rect.max.x, screen_y + self.zoom_y)
             );
             painter.rect_stroke(row_rect, 0.0, (1.0, crate::ui::theme::THEME.grid_line), egui::StrokeKind::Middle); // Horizontal divider
             
             // --- DRAW CLIPS ---
             // Use index loop to avoid immutable borrow of tracks[i] preventing mutation later
             let clip_count = tracks[i].arrangement.clips.len();
             for clip_idx in 0..clip_count {
                 // 1. Read Clip Data (Immutable Scope)
                 let (clip_name, clip_start, clip_len) = {
                     let c = &tracks[i].arrangement.clips[clip_idx];
                     (c.name.clone(), c.start_time.samples, c.length.samples)
                 };

                // Draw Clip
                let sample_to_beats = |s: u64| -> f32 {
                     (s as f64 * (bpm as f64 / (60.0 * sample_rate as f64))) as f32
                };
                 
                let start_beat = sample_to_beats(clip_start);
                let len_beats = sample_to_beats(clip_len);
                 
                let clip_x = grid_rect.min.x + (start_beat * self.zoom_x) - self.scroll_x;
                let clip_w = len_beats * self.zoom_x;
                 
                let clip_rect = egui::Rect::from_min_size(
                    egui::pos2(clip_x, screen_y + 2.0),
                    egui::vec2(clip_w, self.zoom_y - 4.0)
                );
                 
                // Visibility check X
                if clip_rect.max.x < grid_rect.min.x || clip_rect.min.x > grid_rect.max.x {
                    continue;
                }

                // --- INTERACTION ---
                let clip_response = ui.interact(clip_rect, ui.id().with(format!("clip_{}_{}", i, clip_name)), egui::Sense::click_and_drag());
                
                // 1. Start Drag
                if clip_response.drag_started() {
                    self.drag_state = Some(DragState {
                        track_index: i,
                        clip_index: clip_idx,
                        original_start_samples: clip_start,
                        start_mouse_x: ui.input(|inp| inp.pointer.interact_pos().unwrap_or(egui::Pos2::ZERO).x),
                    });
                }
                
                // 2. Handle Dragging
                if let Some(drag) = self.drag_state {
                    if drag.track_index == i && drag.clip_index == clip_idx {
                        if ui.input(|inp| inp.pointer.primary_down()) {
                            let current_mouse_x = ui.input(|inp| inp.pointer.interact_pos().unwrap_or(egui::Pos2::ZERO).x);
                            let delta_x = current_mouse_x - drag.start_mouse_x;
                            
                            // Convert delta pixels -> delta samples
                            let beats_per_sample = bpm as f64 / (60.0 * sample_rate as f64);
                            let samples_per_pixel = (1.0 / self.zoom_x as f64) / beats_per_sample;
                            let delta_samples = (delta_x as f64 * samples_per_pixel) as i64;
                            
                            // Calculate new start time
                            let new_start = (drag.original_start_samples as i64 + delta_samples).max(0) as u64;
                            
                            // Apply to UI (Immediate Feedback)
                            if let Some(c) = tracks[i].arrangement.clips.get_mut(clip_idx) {
                                c.start_time.samples = new_start;
                            }
                            
                            // Sync with Engine (Throttle this? Or send every frame?)
                            // Send every frame for responsiveness, audio thread handles atomic/mutex mostly.
                            // But MoveClip updates Project struct which might be heavy if locked. 
                            // Engine uses `project` in audio thread? No, `project` is in `start_audio_thread` but `AudioEngine` struct has `project`?
                            // Audio Engine struct *owns* Project. 
                            // The command queue is processed at block start. It's fine.
                            let _ = sender.send(EngineCommand::MoveClip { 
                                track_index: i, 
                                clip_index: clip_idx, 
                                new_start 
                            });
                        } else {
                            // Drag Ended
                            self.drag_state = None;
                        }
                    }
                }
                // 3. Context Menu
                clip_response.context_menu(|ui| {
                    ui.label("Properties");
                    ui.separator();
                    
                    if let Some(c) = tracks[i].arrangement.clips.get_mut(clip_idx) {
                        let mut trigger_stretch = false;
                        
                        if ui.checkbox(&mut c.stretch, "Time Stretch").changed() {
                            trigger_stretch = true;
                        }
                        
                        ui.horizontal(|ui| {
                            ui.label("Orig BPM:");
                            if ui.add(egui::DragValue::new(&mut c.original_bpm).speed(0.1).range(20.0..=300.0)).changed() {
                                trigger_stretch = true;
                            }
                        });
                        
                        // Send command if changed
                        if trigger_stretch {
                             let _ = sender.send(EngineCommand::StretchClip { 
                                 track_index: i, 
                                 clip_index: clip_idx, 
                                 original_bpm: c.original_bpm 
                             });
                        }
                    }
                });


                let is_dragging = self.drag_state.map(|d| d.track_index == i && d.clip_index == clip_idx).unwrap_or(false);
                let color = if is_dragging {
                    egui::Color32::from_rgb(120, 170, 220)
                } else {
                    egui::Color32::from_rgb(100, 150, 200)
                };
                
                // Background
                painter.rect_filled(clip_rect, 4.0, color.gamma_multiply(0.3)); // Transparent bg
                painter.rect_stroke(clip_rect, 4.0, (1.0, egui::Color32::WHITE), egui::StrokeKind::Middle);
                
                // WAVEFORM RENDERING (with Caching)
                if let Some(pool_arc) = audio_pool {
                    // Try to lock (don't block UI)
                    // RCU Load (Lock-Free)
                    let pool_guard = pool_arc.load();
                    let pool = &**pool_guard;
                    
                        let asset_id = if tracks[i].arrangement.clips[clip_idx].stretch { 
                            tracks[i].arrangement.clips[clip_idx].cached_id.unwrap_or(tracks[i].arrangement.clips[clip_idx].source_id)
                        } else {
                            tracks[i].arrangement.clips[clip_idx].source_id
                        };

                        let start_offset = tracks[i].arrangement.clips[clip_idx].start_offset.samples as usize;
                        let length_samples = tracks[i].arrangement.clips[clip_idx].length.samples as usize;
                        let width = clip_rect.width();
                        
                        if width > 0.0 && length_samples > 0 {
                            let samples_per_pixel = (length_samples as f32 / width).max(1.0) as usize;
                            
                            // Check cache
                            let cache_valid = self.waveform_cache.get(&asset_id)
                                .map(|c| c.samples_per_peak == samples_per_pixel && (c.width - width).abs() < 1.0)
                                .unwrap_or(false);
                            
                            // Generate cache if needed
                            if !cache_valid {
                                if let Some(asset) = pool.get_asset(asset_id) {
                                    let data = &asset.data;
                                    let mut peaks = Vec::new();
                                    let mut idx = 0;
                                    
                                    while idx < data.len() {
                                        let chunk_end = (idx + samples_per_pixel).min(data.len());
                                        let first_sample = data[idx];
                                        let mut min_v = first_sample;
                                        let mut max_v = first_sample;
                                        
                                        // Stride for very large chunks
                                        let stride = if samples_per_pixel > 100 { samples_per_pixel / 50 } else { 1 };
                                        for k in (idx..chunk_end).step_by(stride.max(1)) {
                                            let s = data[k];
                                            if s < min_v { min_v = s; }
                                            if s > max_v { max_v = s; }
                                        }
                                        peaks.push((min_v, max_v));
                                        idx += samples_per_pixel;
                                    }
                                    
                                    
                                    eprintln!("[UI] Generated Waveform Cache for Asset {}: width={}, samples_per_pixel={}, peaks_len={}. First peak: ({}, {})", 
                                        asset_id, width, samples_per_pixel, peaks.len(), 
                                        peaks.first().map(|p| p.0).unwrap_or(0.0), 
                                        peaks.first().map(|p| p.1).unwrap_or(0.0)
                                    );
                                    
                                    self.waveform_cache.insert(asset_id, WaveformCache {
                                        _asset_id: asset_id,
                                        samples_per_peak: samples_per_pixel,
                                        peaks,
                                        width,
                                    });
                                }
                            }
                            
                            // Draw from cache
                            if let Some(cache) = self.waveform_cache.get(&asset_id) {
                                let start_peak = start_offset / samples_per_pixel;
                                let num_peaks = (width as usize).min(cache.peaks.len().saturating_sub(start_peak));
                                
                                let center_y = clip_rect.center().y;
                                let height = clip_rect.height();
                                
                                // DEBUG: Throttle log
                                if num_peaks > 0 && start_peak == 0 { 
                                     // static mut LAST_DRAW_LOG: u64 = 0;
                                     // Use simple random skip or just print once per asset?
                                     // Let's print first peak values
                                     // eprintln!("[UI] Drawing Waveform: Asset={}, Peaks={}, First=({}, {})", asset_id, num_peaks, cache.peaks[0].0, cache.peaks[0].1);
                                }

                                for px in 0..num_peaks {
                                    if let Some(&(min_v, max_v)) = cache.peaks.get(start_peak + px) {
                                        let x = clip_rect.min.x + px as f32;
                                        // Scale height!
                                        // If signal is -0.02 to 0.02, and height is 60.
                                        // 0.02 * 60 * 0.45 = 0.54 pixels.
                                        // This is < 1 pixel. might be invisible if antialiased or clamped.
                                        // AUTO-SCALE or normalize?
                                        
                                        // For now, let's boost visual gain implicitly or ensure min height.
                                        let mut y_min = center_y + (min_v * height * 0.45);
                                        let mut y_max = center_y + (max_v * height * 0.45);
                                        
                                        // Ensure at least 1px height if peak exists
                                        if (y_max - y_min).abs() < 1.0 {
                                            y_min = center_y - 0.5;
                                            y_max = center_y + 0.5;
                                        }

                                        painter.line_segment([egui::pos2(x, y_min), egui::pos2(x, y_max)], (1.0, color));
                                    }
                                }
                            }
                        }
                }

                
                painter.text(
                    clip_rect.left_center() + egui::vec2(5.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &clip_name,
                    egui::FontId::proportional(12.0),
                    egui::Color32::BLACK
                );
             }
        }
        
        // --- DRAW PLAYHEAD ---
        // Global Sample Pos -> Beats -> X
        // We reuse the calculation:
        let samples_to_beats = |s: u64| -> f32 {
             (s as f64 * (bpm as f64 / (60.0 * sample_rate as f64))) as f32
        };
        let current_beat = samples_to_beats(playback_pos_samples);
        let playhead_x = grid_rect.min.x + (current_beat * self.zoom_x) - self.scroll_x;
        
        if playhead_x >= grid_rect.min.x && playhead_x <= grid_rect.max.x {
             painter.line_segment(
                 [egui::pos2(playhead_x, ruler_rect.bottom()), egui::pos2(playhead_x, content_rect.max.y)],
                 (2.0, egui::Color32::YELLOW)
             );
             
             // Triangle Head
             let head_size = 10.0;
             painter.add(egui::Shape::convex_polygon(
                 vec![
                     egui::pos2(playhead_x, ruler_rect.bottom()),
                     egui::pos2(playhead_x - head_size/2.0, ruler_rect.bottom()),
                     egui::pos2(playhead_x + head_size/2.0, ruler_rect.bottom()),
                     egui::pos2(playhead_x, ruler_rect.bottom() + head_size),
                 ],
                 egui::Color32::YELLOW,
                 egui::Stroke::NONE
             ));
        }

    }
}

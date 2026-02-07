# Omni - DAW / Audio Workstation

## 📋 Opis Projektu
Omni to DAW (Digital Audio Workstation) napisany w Rust. Łączy host audio (UI) z silnikiem audio i obsługą pluginów CLAP przez IPC/shared memory.

## 🏗️ Struktura Workspace

```
omni/
├── omni_host/       # Główna aplikacja GUI (eframe/egui)
├── omni_engine/     # Silnik audio (przetwarzanie, graf, sekwencer)
├── omni_plugin_host/# Wrapper CLAP, ładowanie pluginów
├── omni_shared/     # Współdzielone typy (IPC, projekt, skale)
└── dummy_plugin/    # Plugin testowy
```

## 🔑 Kluczowe Pliki

### omni_host/src/
| Plik | Rola |
|------|------|
| `main.rs` | Główna pętla UI, `OmniApp`, obsługa komend |
| `sequencer_ui.rs` | UI sekwencera krokowego (pitch, velocity, gate, modulation) |
| `arrangement_ui.rs` | UI widoku aranżacji (timeline, klipy) |
| `ui/session.rs` | Session View (sceny, klipy) |
| `ui/piano_roll.rs` | Edytor MIDI/piano roll |
| `ui/mixer.rs` | Mikser (volume, pan) |
| `ui/device.rs` | Panel urządzenia/pluginu |
| `project_io.rs` | Zapis/odczyt projektów |

### omni_engine/src/
| Plik | Rola |
|------|------|
| `engine.rs` | `AudioEngine` - główna logika audio |
| `graph.rs` | Graf przetwarzania audio |
| `plugin_node.rs` | Node dla pluginów CLAP |
| `sequencer.rs` | Logika sekwencera |
| `transport.rs` | Transport (play/stop, tempo, pozycja) |
| `commands.rs` | `EngineCommand` enum |
| `mixer.rs` | Miksowanie audio |

### omni_shared/src/
| Plik | Rola |
|------|------|
| `lib.rs` | IPC protokół (`HostCommand`, `PluginEvent`), shared memory |
| `project.rs` | Struktura projektu, `StepSequencerData` |
| `scale.rs` | Skale muzyczne, kwantyzacja |
| `performance.rs` | Performance patterns (Roll, etc.) |

## 🔧 Komendy

```bash
# Budowanie
cargo build --release

# Uruchomienie
cargo run --release -p omni_host

# Sprawdzenie błędów
cargo check
```

## 📐 Wzorce i Konwencje

### Komunikacja Host ↔ Engine
- Kanały `crossbeam_channel` dla komend (`EngineCommand`)
- Engine działa w osobnym wątku audio

### Komunikacja Host ↔ Plugin
- Shared memory (`OmniShmemHeader`) dla audio/zdarzeń
- IPC przez stdin/stdout dla komend (`HostCommand`/`PluginEvent`)
- Plugin jako osobny proces (`omni_host_plugin`)

### UI (egui)
- `OmniApp` implementuje `eframe::App`
- Stan UI w `OmniApp` (tracki, sekwencery, widok)
- Aktualizacje przez `ctx.request_repaint()`

### Sekwencer
- `StepSequencerData`: pitch, velocity, gate, probability, performance
- Kroki 0-31, loop start/end
- Performance patterns: Roll variants (3/5/7)

## 🎹 Transport i Synchronizacja
- Tempo w BPM
- Playhead w beats (sample-accurate)
- CLAP transport info przekazywane do pluginów

## 📁 Format Projektu
- JSON (`Project` struct w `omni_shared/src/project.rs`)
- Zawiera: tracki, klipy arrangement, dane sekwencerów, stany pluginów

## ⚠️ Znane Uwagi
- Plugin procesy (`omni_host_plugin`) muszą być poprawnie zamykane
- Shared memory cleanup przy shutdown
- Sample rate: 48000 Hz domyślnie
- Buffer size: 512 samples

## 🔍 Częste Operacje

### Dodanie nowej komendy do Engine:
1. Dodaj wariant do `EngineCommand` w `omni_engine/src/commands.rs`
2. Obsłuż w `AudioEngine` w `omni_engine/src/engine.rs`
3. Wywołaj z UI w `omni_host/src/main.rs`

### Dodanie nowej lane w sekwencerze:
1. Rozszerz `StepSequencerData` w `omni_shared/src/project.rs`
2. Dodaj UI w `omni_host/src/sequencer_ui.rs`
3. Obsłuż w engine w `omni_engine/src/engine.rs`

### Dodanie nowego widoku UI:
1. Stwórz plik w `omni_host/src/ui/`
2. Dodaj do `omni_host/src/ui/mod.rs`
3. Zintegruj w `OmniApp::update()` w `main.rs`

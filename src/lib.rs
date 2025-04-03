#![no_std]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![warn(
    clippy::complexity,
    clippy::correctness,
    clippy::perf,
    clippy::style,
    clippy::undocumented_unsafe_blocks,
    rust_2018_idioms
)]

use asr::{
    Address, Process,
    file_format::pe,
    future::{next_tick, retry},
    settings::{Gui, Map},
    string::ArrayCString,
    timer::{self, TimerState},
    watcher::Watcher
};

asr::async_main!(stable);
asr::panic_handler!();

#[derive(Gui)]
struct Settings {
    #[default = true]
    Full_game_run: bool,
    #[default = false]
    Individual_level: bool,
    #[default = false]
    Slow_PC_mode: bool
}

#[derive(Default)]
struct Watchers {
    startByte: Watcher<u8>,
    loadByte: Watcher<u8>,
    level: Watcher<ArrayCString<8>>,
    warRecord: Watcher<ArrayCString<21>>,
    briefingByte: Watcher<u8>,
    mcByte: Watcher<u16>,
    fpsFloat: Watcher<f32>
}

struct Memory {
    start: Address,
    load: Address,
    level: Address,
    warRecord: Address,
    briefing: Address,
    mc: Address,
    fps: Address
}

impl Memory {
    async fn init(process: &Process) -> Self {
        let baseModule = retry(|| process.get_module_address("SniperElite.exe")).await;
        let baseModuleSize = retry(|| pe::read_size_of_image(process, baseModule)).await;
        //asr::print_message(&format!("{}", baseModuleSize));

        match baseModuleSize {
            3805184 => Self {
                start: baseModule + 0x2DB89C,
                load: baseModule + 0x320D25,
                level: baseModule + 0x380CE5,
                warRecord: baseModule + 0x394D28,
                briefing: baseModule + 0x333F91,
                mc: baseModule + 0x32B040,
                fps: baseModule + 0x2E60D0
            },
            _ => Self {
                start: baseModule + 0x35DAD4,
                load: baseModule + 0x3A35A9,
                level: baseModule + 0x418EED,
                warRecord: baseModule + 0x418AA8,
                briefing: baseModule + 0x3B7299,
                mc: baseModule + 0x3AE2E0,
                fps: baseModule + 0x368390
            }
        }
    }
}

fn start(watchers: &Watchers) -> bool {
    watchers.briefingByte.pair.is_some_and(|val|
        val.current == 1
        && watchers.fpsFloat.pair.is_some_and(|val| val.current < 10000.0)
    )
    || watchers.loadByte.pair.is_some_and(|val|
        val.changed_from_to(&0, &1)
        && watchers.fpsFloat.pair.is_some_and(|val| val.current != 60.0)
        && watchers.warRecord.pair.is_some_and(|val| val.current.matches("\\splash\\Loadbar.dds"))
    )
}

fn isWarRecord(watchers: &Watchers) -> bool {
    watchers.fpsFloat.pair.is_some_and(|val| 
        val.current != val.old
        && val.old == 60.0
        && watchers.warRecord.pair.is_some_and(|val| val.current.matches("\\splash\\oldmenu1.dds"))
    )
}

fn leftWarRecord(watchers: &Watchers) -> bool {
    watchers.warRecord.pair.is_some_and(|val| val.current.matches("\\splash\\loading\\level"))
    || watchers.warRecord.pair.is_some_and(|val| val.current.matches("\\splash\\frontsc2.dds"))
}

fn isLoading(watchers: &Watchers) -> Option<bool> {
    Some(
        watchers.loadByte.pair?.current == 0
        && (watchers.briefingByte.pair?.current != 1 && watchers.fpsFloat.pair?.current < 10000.0)
        || watchers.fpsFloat.pair?.current > 10000.0
    )
}

fn split(watchers: &Watchers, settings: &Settings) -> bool {
    match settings.Individual_level {
        true => watchers.startByte.pair.is_some_and(|val| val.current == 5)
        && watchers.mcByte.pair.is_some_and(|val| val.current == 256),
        false => watchers.level.pair.is_some_and(|val|
            val.changed()
            || val.current.matches("level08d")
            && watchers.startByte.pair.is_some_and(|val|
                val.old == 5 && val.current == 2
            )
            || val.current.matches("level02a")
            && watchers.startByte.pair.is_some_and(|val| val.current == 5)
            && watchers.mcByte.pair.is_some_and(|val| val.current == 256)
        )
    }
}

fn mainLoop(process: &Process, memory: &Memory, watchers: &mut Watchers) {
    watchers.startByte.update_infallible(process.read(memory.start).unwrap_or_default());

    watchers.loadByte.update_infallible(process.read(memory.load).unwrap_or(1));

    watchers.briefingByte.update_infallible(process.read(memory.briefing).unwrap_or_default());
    watchers.mcByte.update_infallible(process.read(memory.mc).unwrap_or_default());
    watchers.fpsFloat.update_infallible(process.read(memory.fps).unwrap_or_default());

    watchers.level.update_infallible(process.read(memory.level).unwrap_or_default());
    watchers.warRecord.update_infallible(process.read(memory.warRecord).unwrap_or_default());
}

async fn main() {
    let mut settings = Settings::register();
    let mut map = Map::load();

    asr::set_tick_rate(60.0);
    let mut tickToggled = false;

    let mut warRec: u8 = 0;
    loop {
        let process = Process::wait_attach("SniperElite.exe").await;

        process.until_closes(async {
            let mut watchers = Watchers::default();
            let memory = Memory::init(&process).await;

            loop {
                settings.update();

                if settings.Full_game_run && settings.Individual_level {
                    map.store();
                }

                if settings.Slow_PC_mode && !tickToggled {
                    asr::set_tick_rate(30.0);
                    map = Map::load();
                    tickToggled = true;
                }
                else if !settings.Slow_PC_mode && tickToggled {
                    asr::set_tick_rate(60.0);
                    map = Map::load();
                    tickToggled = false;
                }

                if [TimerState::Running, TimerState::Paused].contains(&timer::state()) {
                    if isWarRecord(&watchers) {
                        warRec = 1;
                        timer::resume_game_time();
                    }
                    if leftWarRecord(&watchers) {
                        warRec = 0;
                    }

                    match isLoading(&watchers) {
                        Some(true) => if warRec != 1 { timer::pause_game_time() },
                        Some(false) => timer::resume_game_time(),
                        _ => ()
                    }

                    if split(&watchers, &settings) {
                        timer::split();
                    }
                }

                if timer::state().eq(&TimerState::NotRunning) && start(&watchers) {
                    timer::start();
                }

                mainLoop(&process, &memory, &mut watchers);
                next_tick().await;
            }
        }).await;
    }
}
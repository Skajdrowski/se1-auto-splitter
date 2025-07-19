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
    Address32, Process,
    file_format::pe,
    future::{next_tick, retry},
    settings::{Gui},
    string::ArrayCString,
    timer::{self, TimerState},
    watcher::Watcher,
    signature::Signature
};

asr::async_main!(stable);
asr::panic_handler!();

#[derive(Gui)]
struct Settings {
    #[default = false]
    Individual_level: bool,
    #[default = false]
    Slow_PC_mode: bool
}

#[derive(Default)]
struct Watchers {
    startByte: Watcher<u8>,
    loadByte: Watcher<u8>,
    level: Watcher<ArrayCString<3>>,
    warRecord: Watcher<ArrayCString<13>>,
    briefingByte: Watcher<u8>,
    mcByte: Watcher<u16>,
    fpsFloat: Watcher<f32>
}

struct Memory {
    start: Address32,
    load: Address32,
    level: Address32,
    warRecord: Address32,
    briefing: Address32,
    mc: Address32,
    fps: Address32
}

//asr::print_limited::<128>(&format_args!("{}", baseModule));
impl Memory {
    async fn init(process: &Process) -> Self {
        const startSIG: Signature<84> = Signature::new("A3 ?? ?? ?? ?? C7 ?? ?? ?? ?? ?? ?? ?? ?? ?? C7 ?? ?? ?? ?? ?? ?? ?? ?? ?? C6 ?? ?? ?? ?? ?? ?? A3 ?? ?? ?? ?? A3 ?? ?? ?? ?? A3 ?? ?? ?? ?? A3 ?? ?? ?? ?? A3 ?? ?? ?? ?? A3 ?? ?? ?? ?? A3 ?? ?? ?? ?? C3 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 8B");
        const loadSIG: Signature<11> = Signature::new("A0 ?? ?? ?? ?? 84 ?? 74 ?? 56 BE");
        const levelSIG: Signature<18> = Signature::new("8A ?? ?? ?? ?? ?? 88 ?? ?? ?? ?? ?? 40 84 ?? 75 ?? A1");
        const warRecordSIG: Signature<11> = Signature::new("88 ?? ?? ?? ?? ?? 8A ?? 33 ?? 84");
        const briefingSIG: Signature<18> = Signature::new("A0 ?? ?? ?? ?? 53 33 ?? 3A ?? 74 ?? E8 ?? ?? ?? ?? 39");
        const mcSIG: Signature<20> = Signature::new("A0 ?? ?? ?? ?? 50 E8 ?? ?? ?? ?? 83 ?? ?? E8 ?? ?? ?? ?? E8");
        const framerateSIG: Signature<41> = Signature::new("C7 ?? ?? ?? ?? ?? ?? ?? ?? ?? D9 ?? ?? ?? ?? ?? D8 ?? ?? ?? ?? ?? D9 ?? ?? ?? ?? ?? D9 ?? ?? ?? ?? ?? D8 ?? ?? ?? ?? ?? D9");

        let baseModule = retry(|| process.get_module_address("SniperElite.exe")).await;
        let baseModuleSize = retry(|| pe::read_size_of_image(process, baseModule)).await;

        let startScan = startSIG.scan_process_range(process, (baseModule, baseModuleSize.into())).unwrap() + 1;
        let loadScan = loadSIG.scan_process_range(process, (baseModule, baseModuleSize.into())).unwrap() + 1;
        let levelScan = levelSIG.scan_process_range(process, (baseModule, baseModuleSize.into())).unwrap() + 2;
        let warRecordScan = warRecordSIG.scan_process_range(process, (baseModule, baseModuleSize.into())).unwrap() + 2;
        let briefingScan = briefingSIG.scan_process_range(process, (baseModule, baseModuleSize.into())).unwrap() + 1;
        let mcScan = mcSIG.scan_process_range(process, (baseModule, baseModuleSize.into())).unwrap() + 1;
        let framerateScan = framerateSIG.scan_process_range(process, (baseModule, baseModuleSize.into())).unwrap() + 2;

        Self {
            start: process.read::<u32>(startScan).unwrap().into(),
            load: process.read::<u32>(loadScan).unwrap().into(),
            level: (process.read::<u32>(levelScan).unwrap() + 0x12).into(),
            warRecord: (process.read::<u32>(warRecordScan).unwrap() + 0x8).into(),
            briefing: process.read::<u32>(briefingScan).unwrap().into(),
            mc: process.read::<u32>(mcScan).unwrap().into(),
            fps: process.read::<u32>(framerateScan).unwrap().into()
        }
    }
}

fn start(watchers: &Watchers) -> bool {
    let fpsFloat = watchers.fpsFloat.pair.unwrap();
    
    watchers.briefingByte.pair.unwrap().current == 1
    && fpsFloat.current < 10000.0
    || watchers.loadByte.pair.unwrap().changed_from_to(&0, &1)
    && fpsFloat.current != 60.0
    && watchers.warRecord.pair.unwrap().current.matches("Loadbar.dds")
}

fn isWarRecord(watchers: &Watchers) -> bool {
    let fpsFloat = watchers.fpsFloat.pair.unwrap();

    fpsFloat.current != fpsFloat.old
    && fpsFloat.old == 60.0
    && watchers.warRecord.pair.unwrap().current.matches("oldmenu1.dds")
}

fn leftWarRecord(watchers: &Watchers) -> bool {
    let warRecord = watchers.warRecord.pair.unwrap();

    warRecord.current.matches("loading\\level")
    || warRecord.current.matches("frontsc2.dds")
}

fn isLoading(watchers: &Watchers) -> Option<bool> {
    Some(
        watchers.loadByte.pair?.current == 0
        && watchers.briefingByte.pair?.current != 1
        || watchers.fpsFloat.pair?.current > 10000.0
    )
}

fn split(watchers: &Watchers, settings: &Settings) -> bool {
    match settings.Individual_level {
        true => watchers.startByte.pair.unwrap().current == 5
        && watchers.mcByte.pair.unwrap().current == 256,
        false => {
            let level = watchers.level.pair.unwrap();

            level.changed()
            && !level.current.matches("02a")
            || level.current.matches("08d")
            && watchers.startByte.pair.unwrap().changed_from_to(&5, &2)
            || level.current.matches("02a")
            && watchers.startByte.pair.unwrap().current == 5
            && watchers.mcByte.pair.unwrap().current == 256
        }
    }
}

fn mainLoop(process: &Process, memory: &Memory, watchers: &mut Watchers) {
    watchers.startByte.update_infallible(process.read(memory.start).unwrap_or(0));

    watchers.loadByte.update_infallible(process.read(memory.load).unwrap_or(1));

    watchers.briefingByte.update_infallible(process.read(memory.briefing).unwrap_or(0));
    watchers.mcByte.update_infallible(process.read(memory.mc).unwrap_or(0));
    watchers.fpsFloat.update_infallible(process.read(memory.fps).unwrap_or(0.0));

    watchers.level.update_infallible(process.read(memory.level).unwrap_or_default());
    watchers.warRecord.update_infallible(process.read(memory.warRecord).unwrap_or_default());
}

async fn main() {
    let mut settings = Settings::register();

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

                if settings.Slow_PC_mode && !tickToggled {
                    asr::set_tick_rate(30.0);
                    tickToggled = true;
                }
                else if !settings.Slow_PC_mode && tickToggled {
                    asr::set_tick_rate(60.0);
                    tickToggled = false;
                }

                mainLoop(&process, &memory, &mut watchers);

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

                next_tick().await;
            }
        }).await;

        if timer::state().eq(&TimerState::Running) {
            timer::pause_game_time();
        }
    }
}
//! Writes a copy of a module where only one voice keeps its notes: isolate module.mod voice out.mod
//! The other voices only keep their flow effects (Bxx, Dxx, Fxx, E6x, EEx), so the song follows
//! the same path. Used to compare renders voice by voice.
use smpltrckr::format::protracker;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, voice, output] = &args[..] else {
        panic!("usage: isolate module.mod voice out.mod");
    };
    let keep: usize = voice.parse::<usize>().unwrap() - 1;
    let mut song = protracker::read(&std::fs::read(input).unwrap()).unwrap();
    for cell in song
        .patterns
        .iter_mut()
        .flat_map(|p| p.rows.iter_mut())
        .flat_map(|r| r.iter_mut().enumerate())
    {
        let (v, cell) = cell;
        if v == keep {
            continue;
        }
        let flow = matches!(cell.effect, 0xB | 0xD | 0xF)
            || (cell.effect == 0xE && matches!(cell.param >> 4, 0x6 | 0xE));
        cell.period = 0;
        cell.sample = 0;
        if !flow {
            cell.effect = 0;
            cell.param = 0;
        }
    }
    std::fs::write(output, protracker::write(&song)).unwrap();
}

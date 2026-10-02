//! Écrit une copie d'un module où seule une voie garde ses notes : isolate module.mod voie sortie.mod
//! Les autres voies ne gardent que leurs effets de déroulement (Bxx, Dxx, Fxx, E6x, EEx),
//! pour que le morceau suive le même chemin. Sert à comparer les rendus voie par voie.
use smpltrckr::format::protracker;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, voice, output] = &args[..] else {
        panic!("usage : isolate module.mod voie sortie.mod");
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

//! Plain-language help for ProTracker effects: what the effect under the cursor does, with
//! its actual values, and the palette of effects to pick from.

use rust_i18n::t;

/// One line describing an effect and its parameter, e.g. "A04: volume down by 4 per tick".
/// `None` for an empty effect column.
pub fn describe(effect: u8, param: u8) -> Option<String> {
    if effect == 0 && param == 0 {
        return None;
    }
    let (x, y) = (param >> 4, param & 0x0F);
    let slide = || {
        if x > 0 {
            t!("effect.slide_up", amount = x).into_owned()
        } else {
            t!("effect.slide_down", amount = y).into_owned()
        }
    };
    let text = match (effect, x) {
        (0x0, _) => t!("effect.arpeggio", x = x, y = y),
        (0x1, _) => t!("effect.porta_up", amount = param),
        (0x2, _) => t!("effect.porta_down", amount = param),
        (0x3, _) if param == 0 => t!("effect.glide_previous"),
        (0x3, _) => t!("effect.glide", speed = param),
        (0x4, _) => t!("effect.vibrato", speed = x, depth = y),
        (0x5, _) => t!("effect.glide_slide", slide = slide()),
        (0x6, _) => t!("effect.vibrato_slide", slide = slide()),
        (0x7, _) => t!("effect.tremolo", speed = x, depth = y),
        (0x8, _) => t!("effect.ignored", name = t!("effect.name.8")),
        (0x9, _) => t!("effect.offset", bytes = param as u32 * 256),
        (0xA, _) => t!("effect.volume_slide", slide = slide()),
        (0xB, _) => t!("effect.jump", position = param),
        (0xC, _) => t!("effect.volume", volume = param.min(64)),
        (0xD, _) => t!("effect.break", row = (x * 10 + y).min(63)),
        (0xE, 0x0) => t!("effect.ignored", name = t!("effect.name.E0")),
        (0xE, 0x1) => t!("effect.fine_up", amount = y),
        (0xE, 0x2) => t!("effect.fine_down", amount = y),
        (0xE, 0x3) => t!("effect.ignored", name = t!("effect.name.E3")),
        (0xE, 0x4) => t!("effect.vibrato_wave", wave = wave_name(y)),
        (0xE, 0x5) => t!("effect.finetune", value = ((y as i8) << 4) >> 4),
        (0xE, 0x6) if y == 0 => t!("effect.loop_start"),
        (0xE, 0x6) => t!("effect.loop", times = y),
        (0xE, 0x7) => t!("effect.tremolo_wave", wave = wave_name(y)),
        (0xE, 0x9) => t!("effect.retrigger", ticks = y),
        (0xE, 0xA) => t!("effect.fine_volume_up", amount = y),
        (0xE, 0xB) => t!("effect.fine_volume_down", amount = y),
        (0xE, 0xC) => t!("effect.cut", tick = y),
        (0xE, 0xD) => t!("effect.delay", tick = y),
        (0xE, 0xE) => t!("effect.repeat_row", times = y),
        (0xE, _) => t!("effect.ignored", name = format!("E{x:X}")),
        (0xF, _) if param == 0 => t!("effect.stop"),
        (0xF, _) if param < 0x20 => t!("effect.speed", speed = param),
        _ => t!("effect.tempo", bpm = param),
    };
    Some(format!("{effect:X}{param:02X}: {text}"))
}

/// Vibrato and tremolo waveforms (E4x, E7x).
fn wave_name(y: u8) -> String {
    let base = match y & 3 {
        1 => t!("effect.wave.ramp"),
        2 => t!("effect.wave.square"),
        _ => t!("effect.wave.sine"),
    };
    if y & 4 != 0 {
        t!("effect.wave.no_reset", wave = base).into_owned()
    } else {
        base.into_owned()
    }
}

/// Effects to pick from, as (digit, short name) pairs.
pub fn palette() -> Vec<(&'static str, String)> {
    [
        "0", "1", "2", "3", "4", "5", "6", "7", "9", "A", "B", "C", "D", "E", "F",
    ]
    .into_iter()
    .map(|digit| (digit, t!(format!("effect.name.{digit}")).into_owned()))
    .collect()
}

/// Extended effects (Ex) to pick from, shown when the effect column holds an E.
pub fn extended_palette() -> Vec<(&'static str, String)> {
    ["1", "2", "4", "5", "6", "7", "9", "A", "B", "C", "D", "E"]
        .into_iter()
        .map(|digit| (digit, t!(format!("effect.name.E{digit}")).into_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_effects_with_their_values() {
        assert_eq!(describe(0, 0), None);
        assert_eq!(
            describe(0xA, 0x04).unwrap(),
            "A04: volume slide: down by 4 per tick"
        );
        assert_eq!(
            describe(0x0, 0x37).unwrap(),
            "037: arpeggio: note, +3 and +7 semitones"
        );
        assert_eq!(describe(0xC, 0x20).unwrap(), "C20: volume 32 (of 64)");
        assert_eq!(describe(0xF, 0x8C).unwrap(), "F8C: tempo 140 BPM");
        assert_eq!(describe(0xE, 0xC3).unwrap(), "EC3: cut the note at tick 3");
        assert!(describe(0x8, 0x80).unwrap().contains("ignored"));
    }

    #[test]
    fn every_effect_has_a_name_in_every_language() {
        for lang in crate::lang::LANGUAGES {
            for (digit, _) in palette() {
                let key = format!("effect.name.{digit}");
                assert_ne!(t!(&key, locale = lang), key, "{lang}: {key}");
            }
            for (digit, _) in extended_palette() {
                let key = format!("effect.name.E{digit}");
                assert_ne!(t!(&key, locale = lang), key, "{lang}: {key}");
            }
        }
    }
}

//! Cheat sheet for the agent: ProTracker effects, notes, scales, chords, and a how-to.

pub const TOPICS: [&str; 5] = ["guide", "effects", "notes", "scales", "chords"];

pub fn get(topic: &str) -> Option<&'static str> {
    Some(match topic {
        "guide" => GUIDE,
        "effects" => EFFECTS,
        "notes" => NOTES,
        "scales" => SCALES,
        "chords" => CHORDS,
        _ => return None,
    })
}

const GUIDE: &str = "\
# Composing with smpltrckr

A .mod song: 4 voices, 31 samples, 64-row patterns, and an order list that tells in which
order the patterns play (a pattern may come back several times).

## Time
- Speed (ticks per row) and tempo (BPM): speed 6 and 125 BPM by default, i.e. 120 ms per
  row; 16 rows = one 4/4 bar at 125 BPM (one row = a sixteenth note).
- Fxx changes the speed (F01 to F1F) or the tempo (F20 to FFF); put it on row 00 of the
  first pattern, on any voice. F00 stops the song. Easiest: `song_set_tempo`.
- Row duration = speed × 2.5 / BPM seconds.

## Cells
`C-3 01 A04`: note, sample (01 to 31, decimal), effect (3 hex digits).
`...` = nothing. A note without a sample keeps the voice's previous sample (and its volume).
A row is written `NN | voice 1 cell | voice 2 cell | ...`.
A note lasts until the next note on the same voice, or the end of the sample.
To stop a looping note: EC0 (cut), C00 (volume 0), or a next note.

## Pitch
Notes from C-1 to B-3 (ProTracker's 3 octaves). The actual pitch depends on the sample:
- generated waveforms (32-byte cycle): C-1 ≈ C3, C-2 ≈ C4 (middle C), C-3 ≈ C5;
  with a 64-byte cycle everything drops an octave (handy for basses);
- generated drums (kick, snare, hihat): play them at C-3;
- imported samples: `sample_load` tells which note gives the original pitch.

## Suggested method
1. `song_new`, then generate or load samples (`sample_generate`, `sample_load`).
2. Write a rhythm pattern (voices 1 and 2), a bass line (voice 3), a melody or arpeggiated
   chords (voice 4, effect 0xy).
3. Vary: copy a pattern (`pattern_copy`), change it, transpose it (`pattern_transpose`).
4. Chain the patterns with `order_set`, check with `song_info` (duration), then
   `render_wav` and `song_save`.
5. Everything can be undone with `undo`.

## Mixing
`mix_set` mutes, solos or sets the volume of a voice for the session and WAV renders; it is
not saved in the .mod. For a lasting volume: the sample volume, or effect Cxx.

## Live sessions
When `ping` says live, the user has the song open in their editor and edits it with you.
- `song_info` tells where they are (position, pattern, row, voice, playing or not).
- Your changes show up on their screen at once, highlighted in green for a while; they
  hear them on the next play, even while the song is playing.
- Work alongside them: prefer voices or patterns they are not on, and say what you did.
- `undo` undoes the last change of anyone, theirs included: use it for your own changes only.
- `song_new` and `song_load` are refused while they have unsaved changes.
";

const EFFECTS: &str = "\
# ProTracker effects (3 hex digits: effect x y, or effect xx)

0xy  Arpeggio: cycles note, note+x semitones, note+y semitones on every tick (chords).
1xx  Portamento up by xx period units per tick (after tick 0).
2xx  Portamento down.
3xx  Portamento to the written note, speed xx (00 = previous speed). The note does not
     restart: used for glides. With 3xx, the note is a target.
4xy  Vibrato: speed x, depth y (0 = previous value).
5xy  Portamento to note (3, previous speed) + volume slide (like Axy).
6xy  Vibrato (4, previous values) + volume slide (like Axy).
7xy  Tremolo: speed x, depth y.
9xx  Start the sample at xx × 256 bytes.
Axy  Volume slide on every tick: +x, or -y when x = 0.
Bxx  Jump to position xx of the order list (hex).
Cxx  Voice volume (00 to 40 hex, i.e. 0 to 64).
Dxy  Pattern break: jump to row x×10+y (decimal) of the next position.
E1x  Fine portamento up by x (once).
E2x  Fine portamento down by x.
E4x  Vibrato waveform: 0 sine, 1 ramp, 2 square (+4: do not reset).
E5x  Note finetune (0 to 7, then 8 to F = -8 to -1).
E60  Pattern loop start; E6x: play again x times from the loop start.
E7x  Tremolo waveform.
E9x  Retrigger the note every x ticks.
EAx  Volume +x (once).
EBx  Volume -x (once).
ECx  Cut the note at tick x.
EDx  Delay the note to tick x.
EEx  Repeat the row x times (notes are not played again).
Fxx  Speed (01 to 1F) or tempo in BPM (20 to FF). F00 stops the song.

Ignored by smpltrckr: 8xx (panning), E0x (Amiga filter), E3x (glissando), EFx.
";

const NOTES: &str = "\
# Notes and pitch

Names: C- C# D- D# E- F- F# G- G# A- A# B-, followed by octave 1, 2 or 3 (e.g. C#2).
No flats: Db = C#, Eb = D#, Gb = F#, Ab = G#, Bb = A#.

With a 32-byte cycle sample (generated waveforms):
  C-1 ≈ 130 Hz (C3)   C-2 ≈ 259 Hz (C4)   C-3 ≈ 518 Hz (C5)   B-3 ≈ 979 Hz
With a 64-byte cycle: one octave lower. With 16 bytes: one octave higher.
A sample played at C-3 is read at 16,574 bytes per second, at C-2 at 8,287.

Transpose by n semitones: `pattern_transpose`. One octave = 12 semitones.
";

const SCALES: &str = "\
# Scales (semitones from the tonic)

major              0 2 4 5 7 9 11     e.g. C: C D E F G A B
natural minor      0 2 3 5 7 8 10     e.g. A: A B C D E F G
harmonic minor     0 2 3 5 7 8 11     e.g. A: A B C D E F G#
dorian             0 2 3 5 7 9 10     e.g. D: D E F G A B C
phrygian           0 1 3 5 7 8 10     e.g. E: E F G A B C D
lydian             0 2 4 6 7 9 11     e.g. F: F G A B C D E
mixolydian         0 2 4 5 7 9 10     e.g. G: G A B C D E F
major pentatonic   0 2 4 7 9          e.g. C: C D E G A
minor pentatonic   0 3 5 7 10         e.g. A: A C D E G
blues              0 3 5 6 7 10       e.g. A: A C D D# E G

Common progressions (degrees): I-V-vi-IV, i-VI-III-VII, ii-V-I, i-iv-v, I-IV-V.
";

const CHORDS: &str = "\
# Chords as arpeggios (effect 0xy)

A chord plays as an arpeggio on a single voice: root note + effect 0xy, where x and y are
the intervals in semitones (hex) of the other two notes.

major        047   (root, major third, fifth)
minor        037
diminished   036
augmented    048
sus2         027
sus4         057
seventh      04A (no fifth: third and minor seventh)
maj7         04B
min7         03A
power chord  07C (fifth and octave)

E.g. `A-2 05 037` plays an A minor; `F-2 05 047` an F major.
Arpeggios sound best with a speed of 3 to 6 and a looped sample (pulse, square).
";

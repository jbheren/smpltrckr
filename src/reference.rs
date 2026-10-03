//! Aide-mémoire pour l'agent : effets ProTracker, notes, gammes, accords, mode d'emploi.

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
# Composer avec smpltrckr

Un morceau .mod : 4 voies, 31 samples, des patterns de 64 lignes, et une liste d'ordre qui
dit dans quel ordre jouer les patterns (un pattern peut revenir plusieurs fois).

## Le temps
- Vitesse (ticks par ligne) et tempo (BPM) : par défaut vitesse 6 et 125 BPM, soit 120 ms par
  ligne, 16 lignes = une mesure de 4 temps à 125 BPM (une ligne = une double croche).
- Fxx change la vitesse (F01 à F1F) ou le tempo (F20 à FFF) ; à placer en ligne 00 du premier
  pattern, sur n'importe quelle voie. F00 arrête le morceau. Le plus simple : `song_set_tempo`.
- Durée d'une ligne = vitesse × 2,5 / BPM secondes.

## Les cellules
`C-3 01 A04` : note, sample (01 à 31, en décimal), effet (3 chiffres hexadécimaux).
`...` = rien. Une note sans sample garde le sample précédent de la voie (et son volume).
Une ligne s'écrit `NN | cellule voie 1 | cellule voie 2 | ...`.
Une note tient jusqu'à la note suivante de la même voie, ou jusqu'à la fin du sample.
Pour arrêter une note qui boucle : EC0 (coupe), C00 (volume 0), ou une note suivante.

## Les hauteurs
Notes de C-1 à B-3 (3 octaves ProTracker). La hauteur réelle dépend du sample :
- formes d'onde générées (cycle de 32 octets) : C-1 ≈ do 3, C-2 ≈ do 4, C-3 ≈ do 5 ;
  avec un cycle de 64 octets, tout descend d'une octave (utile pour les basses) ;
- percussions générées (kick, snare, hihat) : à jouer en C-3 ;
- samples importés : `sample_load` indique la note qui rend la hauteur d'origine.

## Méthode conseillée
1. `song_new`, puis générer ou charger les samples (`sample_generate`, `sample_load`).
2. Écrire un pattern de rythme (voie 1 et 2), une basse (voie 3), une mélodie ou des
   accords en arpège (voie 4, effet 0xy).
3. Varier : copier un pattern (`pattern_copy`), le modifier, transposer (`pattern_transpose`).
4. Enchaîner les patterns avec `order_set`, vérifier avec `song_info` (durée), puis
   `render_wav` et `song_save`.
5. Tout s'annule avec `undo`.

## Le mixage
`mix_set` coupe, met en solo ou règle le volume d'une voie pour la session et le rendu WAV ;
il n'est pas enregistré dans le .mod. Pour un volume permanent : volume du sample, ou effet Cxx.
";

const EFFECTS: &str = "\
# Effets ProTracker (3 chiffres hexadécimaux : effet x y, ou effet xx)

0xy  Arpège : alterne note, note+x demi-tons, note+y demi-tons à chaque tick (accords).
1xx  Portamento vers le haut de xx unités de période par tick (après le tick 0).
2xx  Portamento vers le bas.
3xx  Portamento vers la note écrite, vitesse xx (00 = vitesse précédente). La note ne
     redémarre pas : sert aux glissés. Avec 3xx, la note est une cible.
4xy  Vibrato : vitesse x, profondeur y (0 = valeur précédente).
5xy  Portamento vers la note (3, vitesse précédente) + glissement de volume (comme Axy).
6xy  Vibrato (4, valeurs précédentes) + glissement de volume (comme Axy).
7xy  Trémolo : vitesse x, profondeur y.
9xx  Départ dans le sample à xx × 256 octets.
Axy  Glissement de volume à chaque tick : +x, ou -y si x = 0.
Bxx  Saute à la position xx de la liste d'ordre (en hexadécimal).
Cxx  Volume de la voie (00 à 40 en hexadécimal, soit 0 à 64).
Dxy  Fin du pattern : saute à la ligne x×10+y (en décimal) de la position suivante.
E1x  Portamento fin vers le haut de x (une seule fois).
E2x  Portamento fin vers le bas de x.
E4x  Forme du vibrato : 0 sinus, 1 rampe, 2 carré (+4 : ne pas réinitialiser).
E5x  Finetune de la note (0 à 7, puis 8 à F = -8 à -1).
E60  Début de boucle de pattern ; E6x : rejoue x fois depuis le début de boucle.
E7x  Forme du trémolo.
E9x  Relance la note tous les x ticks.
EAx  Volume +x (une seule fois).
EBx  Volume -x (une seule fois).
ECx  Coupe la note au tick x.
EDx  Retarde la note au tick x.
EEx  Répète la ligne x fois (les notes ne sont pas rejouées).
Fxx  Vitesse (01 à 1F) ou tempo en BPM (20 à FF). F00 arrête le morceau.

Ignorés par smpltrckr : 8xx (panoramique), E0x (filtre Amiga), E3x (glissando), EFx.
";

const NOTES: &str = "\
# Notes et hauteurs

Noms : C- C# D- D# E- F- F# G- G# A- A# B-, suivis de l'octave 1, 2 ou 3 (ex. C#2).
Pas de bémols : Db = C#, Eb = D#, Gb = F#, Ab = G#, Bb = A#.

Avec un sample de cycle 32 octets (formes générées) :
  C-1 ≈ 130 Hz (do 3)   C-2 ≈ 259 Hz (do 4)   C-3 ≈ 518 Hz (do 5)   B-3 ≈ 979 Hz
Avec un cycle de 64 octets : une octave plus bas. De 16 octets : une octave plus haut.
Un sample joué en C-3 est lu à 16 574 octets par seconde, en C-2 à 8 287.

Transposer de n demi-tons : `pattern_transpose`. Une octave = 12 demi-tons.
";

const SCALES: &str = "\
# Gammes (demi-tons depuis la tonique)

majeure            0 2 4 5 7 9 11     ex. C : C D E F G A B
mineure naturelle  0 2 3 5 7 8 10     ex. A : A B C D E F G
mineure harmonique 0 2 3 5 7 8 11     ex. A : A B C D E F G#
dorienne           0 2 3 5 7 9 10     ex. D : D E F G A B C
phrygienne         0 1 3 5 7 8 10     ex. E : E F G A B C D
lydienne           0 2 4 6 7 9 11     ex. F : F G A B C D E
mixolydienne       0 2 4 5 7 9 10     ex. G : G A B C D E F
pentatonique maj.  0 2 4 7 9          ex. C : C D E G A
pentatonique min.  0 3 5 7 10         ex. A : A C D E G
blues              0 3 5 6 7 10       ex. A : A C D D# E G

Progressions courantes (degrés) : I-V-vi-IV, i-VI-III-VII, ii-V-I, i-iv-v, I-IV-V.
";

const CHORDS: &str = "\
# Accords et arpèges (effet 0xy)

Un accord se joue en arpège sur une seule voie : note fondamentale + effet 0xy, où x et y
sont les intervalles en demi-tons (hexadécimal) des deux autres notes.

majeur        047   (fondamentale, tierce majeure, quinte)
mineur        037
diminué       036
augmenté      048
sus2          027
sus4          057
septième      04A (sans quinte : tierce et septième mineure)
maj7          04B
min7          03A
quinte seule  07C (quinte et octave, « power chord »)

Ex. : `A-2 05 037` joue un la mineur ; `F-2 05 047` un fa majeur.
L'arpège sonne mieux avec une vitesse de 3 à 6 et un sample bouclé (pulse, square).
";

---
projet: smpltrckr
statut: proposition
date: 2026-10-02
maj: 2026-10-02
source: "[[Tracker texte piloté par agent]]"
---
# smpltrckr — plan de développement (proposition)

Tracker en mode texte façon ProTracker / Amiga, sous Omarchy, qui lit et écrit des `.mod`, et qu'on pilote au clavier ou par un agent IA via MCP.

Ce document est une **proposition à discuter**. Les choix marqués « recommandé » sont des avis, pas des décisions. Les questions encore à trancher par JB sont regroupées à la fin.

## Décisions de JB (02/10/2026)

- **L'agent peut travailler seul.** Le mode autonome (l'agent compose sans la TUI, sur des fichiers, et rend un WAV) devient une priorité. Il passe avant l'interface clavier dans les phases.
- **Rust** est retenu comme langage.
- **La disposition du clavier n'est pas un sujet pour l'instant.** On code des raccourcis simples, et on verra l'AZERTY plus tard.
- **Pas d'émulation de Paula.** Une lecture propre suffit, sans chercher le « grain » exact de l'Amiga.
- **4 voies par défaut.** Un nouveau morceau a 4 voies. On accepte plus de voies seulement quand un `.mod` existant en contient (variantes `6CHN`, `8CHN`…).
- **Les samples sont dans le `.mod`.** Pour composer, on peut charger des fichiers **WAV et AIFF**, comme dans ProTracker ou FT2, en restant simple.
- **XM serait un vrai plus.** Ce n'est pas dans le MVP, mais le modèle de données ne doit pas le rendre impossible.
- **Le nom `smpltrckr` reste**, au moins pour l'instant.
- **On commence avec Claude** comme agent. Un modèle local pourra être testé plus tard (cf. [[Stack IA locale]]).
- **Interface en français, anglais et japonais** (03/10/2026). La langue suit le système (`LANG`…), et `--lang fr|en|ja` ou `SMPLTRCKR_LANG` permettent de l'imposer. Le côté agent (outils MCP, guide, messages) reste en anglais. Le code est commenté en anglais, avec de temps en temps un clin d'œil en français, ton hacker et pirate, sans en faire trop.
- **Le mixage par voie (coupure, solo, volume) vaut pour la session seulement**, puisque le `.mod` ne le stocke pas. Un petit fichier à côté du morceau pourra venir plus tard si le besoin se fait sentir.
- **Dépôt public à la fin, sur le compte GitHub `jbheren`**, avec des commits au nom de JB (voir § 9).

---

## 1. Principes directeurs

1. **Un seul modèle, deux façons de piloter.** Le clavier et l'agent passent par la même couche de *commandes*. Toute action de l'agent est visible en direct dans l'interface et peut être annulée. Toute action au clavier existe aussi pour l'agent.
2. **Le `.mod` est le format de référence.** On charge un `.mod` puis on le réenregistre : le fichier doit être identique à l'octet près. C'est le premier critère de réussite, c'est aussi la condition fixée dans la note pour passer de « graine » à « chantier ».
3. **Du texte partout.** L'écran est du texte, et l'agent aussi lit et écrit les patterns sous forme de texte (notation façon ProTracker : `C-3 01 A04`). Les LLM sont bons avec ce genre de notation.
4. **Petit d'abord.** Un MVP utilisable vite (l'agent compose seul un `.mod` et le rend en WAV), puis on enrichit.
5. **Garder des traces.** L'historique des commandes (humain / agent) est journalisé. Ça servira pour l'épisode [[Transition agentique]].

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     processus smpltrckr                     │
│                                                             │
│  ┌──────────────┐   commandes  ┌──────────────────────────┐ │
│  │ TUI (clavier)│ ───────────► │                          │ │
│  └──────────────┘              │   Cœur : Song + Commandes│ │
│  ┌──────────────┐   commandes  │   (undo/redo, journal)   │ │
│  │ Serveur MCP  │ ───────────► │                          │ │
│  │ (socket loc.)│ ◄─ lecture ─ └────────────┬─────────────┘ │
│  └──────▲───────┘                           │ instantanés   │
│         │                         ┌─────────▼─────────┐     │
│         │                         │ Replayer (thread  │     │
│         │                         │ audio temps réel) │──► PipeWire
│         │                         └───────────────────┘     │
└─────────┼───────────────────────────────────────────────────┘
          │ socket Unix / HTTP localhost
┌─────────┴──────────┐
│ `smpltrckr mcp`    │ ◄── stdio ── Claude Code / agent local
│ (pont stdio, ou    │
│  mode sans TUI)    │
└────────────────────┘
```

### Modules

| Module | Rôle |
|---|---|
| `format` | Lecture/écriture `.mod` (M.K., M!K!, FLT4, 4CHN… ; 15 samples Soundtracker en lecture seule au début). Conversion vers/depuis la notation texte. Aucune dépendance audio. |
| `song` | Modèle en mémoire : titre, samples, liste d'ordre, patterns de N lignes × N voies. Pour le `.mod` : 31 samples, 128 positions, 64 lignes, 4 voies par défaut. Le modèle est juste assez large pour accueillir le XM plus tard (voir plus bas). |
| `commands` | Toutes les modifications passent par des commandes (`SetCell`, `InsertRow`, `SetOrder`, `LoadSample`, …) avec undo/redo et une étiquette d'origine (clavier / agent). |
| `replayer` | Moteur de lecture ProTracker : périodes Amiga, finetune, effets 0–F et Exx, tick/tempo/BPM. Sert à la lecture temps réel **et** au rendu WAV hors ligne. |
| `audio` | Sortie son (thread temps réel, sans allocation ni verrou bloquant), alimentée par le replayer. |
| `tui` | Interface terminal : éditeur de patterns, liste d'ordre, liste de samples, transport, oscilloscopes et VU-mètres. |
| `samples` | Import de WAV et d'AIFF, conversion vers le format du `.mod`, générateur de formes d'onde simples. |
| `mcp` | Outils MCP qui traduisent les appels en commandes, plus des ressources (référence des effets, état du morceau). |

### Préparer le XM sans le coder

Pour le MVP, on ne code rien de spécifique au XM. On fait seulement trois choix de modèle qui laissent la porte ouverte :
- le nombre de voies et le nombre de lignes par pattern sont variables, pas fixés à 4 et 64 ;
- les données d'un sample peuvent être en 8 ou 16 bits. Le `.mod` n'utilise que le 8 bits, ce qui ne gêne pas l'aller-retour à l'octet près ;
- une cellule de pattern pourra recevoir plus tard une colonne de volume, propre au XM.

Les instruments XM (enveloppes, plusieurs samples par instrument) et le replayer façon FT2 viendront dans une phase à part. Les règles de lecture de FT2 diffèrent assez de celles de ProTracker pour justifier un second moteur, plutôt qu'un seul moteur bardé de cas particuliers.

### Pourquoi écrire notre propre replayer plutôt que d'utiliser libopenmpt / libxmp

- libopenmpt et libxmp **lisent** un module chargé. Elles ne prévoient pas qu'on modifie un pattern pendant la lecture. Avec elles, chaque modification oblige à réécrire le `.mod` en mémoire et à le recharger. C'est jouable, mais le retour sonore pendant l'édition en souffre.
- Les oscilloscopes et VU-mètres par voie demandent d'accéder à l'état interne de chaque voie.
- Le replayer ProTracker est petit et très bien documenté. pt2-clone (8bitbubsy) en donne une version de référence.
- **libopenmpt reste utile comme oracle de test** : on rend le même module avec les deux moteurs et on compare. Le replayer « maison » peut d'ailleurs venir après : un **plan B** consiste à démarrer avec libopenmpt et sa stratégie de rechargement pour avoir du son tout de suite.

### Comment l'agent se branche (point délicat)

Un client MCP comme Claude Code lance d'habitude son serveur en sous-processus (stdio). Or le tracker tourne déjà, dans le terminal de JB. Proposition :

- Le processus TUI ouvre un **socket local** (Unix ou HTTP sur `localhost`).
- `smpltrckr mcp` est un petit pont stdio. Si une instance TUI tourne, il s'y connecte et l'agent agit **en direct** sur le morceau ouvert. Sinon, il fonctionne **sans TUI**, directement sur des fichiers (pratique pour laisser l'agent composer seul et rendre un WAV).
- Variante plus simple : la TUI expose directement un endpoint MCP en HTTP (streamable HTTP) sur `localhost`. On choisira selon ce que les clients visés supportent le mieux.

---

## 3. Choix techniques et compromis

### Langage et interface

| Option | Pour | Contre |
|---|---|---|
| **Rust + Ratatui** (retenu) | Audio temps réel fiable (pas de ramasse-miettes). Binaire unique. Ratatui est mûr (couleurs 24 bits, Braille, demi-blocs). SDK MCP Rust officiel (`rmcp`). cpal pour le son. Les agents écrivent bien le Rust, et le compilateur rattrape beaucoup d'erreurs. | Courbe d'apprentissage si JB veut relire ou modifier le code à la main. Compilation plus lente. |
| Go + Bubble Tea | Simple, compile vite, SDK MCP Go officiel. nezumi existe comme point de départ. | Le ramasse-miettes peut faire craquer l'audio temps réel (gérable). nezumi dépend de libopenmpt et ne compte que 8 commits : peu à reprendre. |
| Python + Textual | Prototypage le plus rapide, très lisible. SDK MCP Python mûr. | L'audio temps réel en Python est fragile (GIL, latence). Distribution moins propre. Bien pour un prototype, moins pour l'outil final. |

**Décision : Rust + Ratatui + cpal + rmcp**, et on part de zéro. Les projets existants servent de référence, pas de base :
- nezumi : idées d'interface ;
- pt2-clone : comportement exact du replayer ;
- le fork MCP de ft2-clone (mova77) : liste d'outils MCP déjà pensée pour un tracker.

### Son sous Omarchy

Omarchy tourne sous PipeWire. cpal passe par ALSA, et la couche de compatibilité `pipewire-alsa` fait le lien. À vérifier dès la phase 0 : latence obtenue, absence de craquements.

### Fidélité Amiga

Décision : une lecture propre, avec les périodes et le timing de ProTracker et une interpolation simple. Pas d'émulation de Paula ni du filtre de l'A500 : ça ajouterait de la complexité pour un gain modeste.

### Import de samples (WAV, AIFF)

Dans un `.mod`, les samples sont stockés dans le fichier : 8 bits signés, mono, 128 Ko au plus (65 535 mots). On procède comme ProTracker et FT2, en restant simple :
- on lit le WAV ou l'AIFF, en 8, 16, 24 bits ou en flottant, mono ou stéréo ;
- on mixe en mono et on convertit en 8 bits, avec un dithering léger en option ;
- on ne rééchantillonne pas : on garde la fréquence d'origine et on règle la hauteur avec la note jouée, comme sur l'Amiga. Une option « réduire de moitié » aide quand un sample est trop long ;
- si le sample dépasse la limite, on le tronque et on prévient (et l'outil MCP le signale à l'agent) ;
- on récupère le nom du fichier comme nom de sample (22 caractères) et, si le fichier en contient, les points de boucle.

Bibliothèques Rust envisagées : `hound` pour le WAV, et pour l'AIFF `symphonia` ou un petit lecteur maison. Le format AIFF est simple, et la prise en charge de l'AIFF par `symphonia` reste à vérifier.

### Clavier (reporté)

Pour l'instant, on ne s'occupe pas de la disposition du clavier. Les raccourcis seront regroupés dans une seule table, pour qu'on puisse plus tard ajouter une disposition AZERTY ou le protocole clavier de kitty sans toucher au reste.

---

## 4. Notation texte des patterns

C'est le format d'échange avec l'agent. Il peut aussi servir d'export lisible et versionnable avec git :

```
# pattern 00
00 | C-3 01 ... | ... .. ... | E-3 02 C20 | ... .. ...
01 | ... .. ... | ... .. ... | ... .. ... | ... .. ...
02 | D-3 01 A04 | ... .. ... | ... .. ... | ... .. F06
```

Chaque cellule s'écrit `note octave | sample | effet+paramètre`. Les outils MCP accepteront aussi des écritures partielles, ligne par ligne ou cellule par cellule, pour limiter les tokens.

---

## 5. Outils MCP (première esquisse)

| Groupe | Outils |
|---|---|
| Morceau | `song_new`, `song_load`, `song_save`, `song_info`, `song_set_title` |
| Ordre | `order_get`, `order_set` |
| Patterns | `pattern_get` (texte), `pattern_write` (lignes/cellules en texte), `pattern_clear`, `pattern_copy`, `pattern_transpose` |
| Samples | `sample_list`, `sample_load` (WAV, AIFF), `sample_generate` (sinus, carré, scie, bruit, « chip »), `sample_set` (volume, finetune, boucle) |
| Transport | `play`, `play_pattern`, `stop`, `position` |
| Rendu | `render_wav` |
| Historique | `undo`, `redo`, `history` |
| Ressources | référence des effets ProTracker, gammes et accords, état courant du morceau |

L'agent n'entend pas. Une piste pour plus tard : un outil `analyze` qui rend le morceau et en renvoie un résumé chiffré (niveaux par voie, saturation, silences, densité de notes).

---

## 6. Phases et jalons

### Phase 0 — Validation de la pile (1 à 2 sessions)
- Squelette Rust, première fenêtre Ratatui, un son de test via cpal sous Omarchy (latence et craquements).
- Serveur MCP « hello world » avec rmcp, branché à Claude Code (stdio, puis le pont vers un socket).
- **Jalon :** les trois briques (TUI, son, MCP) fonctionnent séparément sur le poste de JB.
- *En option, en parallèle :* tester le fork MCP de ft2-clone pour voir comment un agent s'y prend pour composer. Ce qu'on apprend nourrira la conception des outils MCP.

**Résultats (02/10/2026).** Le jalon est atteint, sauf le pont vers un socket, reporté à la phase 5 où il sert vraiment.
- Pile : Rust 1.99, installé via mise et épinglé dans `mise.toml`. Ratatui 0.30, cpal 0.18, rmcp 3.5.
- `smpltrckr tone` : passe par ALSA, puis par PipeWire grâce à sa couche ALSA. Mesures sur 4 s : 48 kHz en flottant 32 bits, tampons de 512 trames (10,7 ms), latence estimée 32 ms, au plus 245 µs de calcul par callback (version de debug), aucune erreur ni décrochage. Reste à confirmer à l'oreille. cpal 0.18 a aussi un backend PipeWire natif (option `pipewire`), à essayer si la latence devient gênante.
- `smpltrckr ui` : maquette de l'écran (pattern 4 voies qui défile, VU-mètres en blocs de couleur). Un test vérifie le rendu.
- `smpltrckr mcp` : serveur stdio avec deux outils de test (`ping`, `note_info`), déclaré dans `.mcp.json`. Claude Code s'y connecte et appelle les outils.
- Le test optionnel du fork ft2-clone n'a pas été fait.

### Phase 1 — Le format `.mod`
- Parseur et écrivain `.mod`, et conversion vers/depuis la notation texte.
- Outils en ligne de commande : `smpltrckr dump fichier.mod` (texte) et `smpltrckr roundtrip fichier.mod`.
- Corpus de test (quelques dizaines de modules de The Mod Archive, variantes M.K., FLT4, 8CHN).
- **Jalon :** sur tout le corpus, un module chargé puis réenregistré est **identique à l'octet près**. *(C'est la condition de la note pour passer au statut de chantier.)*

**Résultats (02/10/2026).** Jalon atteint.
- Corpus de 50 `.mod` tirés au hasard sur The Mod Archive (`scripts/fetch-corpus.py`). Les modules restent hors du dépôt, à cause de leurs droits ; seuls leurs identifiants sont versionnés dans `tests/corpus-ids.txt`. Le corpus contient 47 `M.K.`, un `8CHN` et un Soundtracker 15 samples. **49 modules sur 49 reviennent identiques à l'octet près** (`smpltrckr roundtrip`, et le test `tests/corpus.rs`). Le 50ᵉ, compressé avec PowerPacker, est refusé avec un message clair.
- Les variantes `M!K!`, `FLT4`, `6CHN`, `8CHN` et `12CH` sont couvertes par des tests synthétiques. `FLT8` est refusé, faute d'exemple réel.
- Pour garantir l'aller-retour exact, le modèle garde tout ce qui est brut : noms sur 22 octets, positions d'ordre inutilisées, samples tronqués, octets en fin de fichier.
- Notation texte : `smpltrckr dump`, plus la lecture d'une cellule, d'une ligne ou d'un pattern, prête pour les outils MCP. Des notes des octaves 0 et 4, écrites par FT2 ou OpenMPT, apparaissent dans 6 modules du corpus. Elles s'affichent (`C-4`) mais ne s'écrivent pas : l'agent reste dans les octaves 1 à 3 de ProTracker.

### Phase 2 — Le replayer
- Replayer ProTracker, rendu WAV hors ligne (`smpltrckr render`).
- Tests de comparaison avec libopenmpt (ou pt2-clone) sur le corpus.
- Mixage par voie : coupure (mute), solo et volume de chaque voie. C'est peu de code dans le replayer, et ça sert à la fois à la TUI, à l'agent et au rendu WAV voie par voie.
- Le replayer publie, sans verrou, l'état de chaque voie (niveau, quelques centaines d'échantillons récents) pour l'affichage.
- **Jalon :** les rendus sont fidèles à l'oreille, et les écarts mesurés restent sous un seuil sur la plupart des modules du corpus.

**Résultats (03/10/2026).** Jalon atteint, à la mesure comme à l'écoute : JB trouve le son « super ».
- Replayer maison (`src/replayer.rs`) : séquenceur, effets 0 à F et Exx (sauf E0, E3, EF et 8xx, ignorés), interpolation linéaire, stéréo à l'Amiga (gauche-droite-droite-gauche) avec séparation réglable. Rien n'est alloué dans le chemin audio.
- Mixage par voie (coupure, solo, volume) et moniteur sans verrou (`src/monitor.rs`) : oscilloscope et niveau de chaque voie, plus le master, prêts pour l'interface.
- Commandes : `smpltrckr play` (sortie audio, VU-mètres en texte) et `smpltrckr render` (WAV stéréo, ou une piste par voie avec `--stems`), avec `--mute`, `--solo`, `--volume`, `--separation`.
- Comparaison avec libopenmpt (`scripts/compare-corpus.sh`), voie par voie, sur les 49 modules lisibles :
  - **durées** : toutes à 0,5 % près, et chaque écart restant est expliqué. libopenmpt ajoute environ 0,1 s en fin de morceau. Il ignore aussi F00, que nous traitons comme un arrêt, comme ProTracker ;
  - **enveloppes** (corrélation d'énergie par fenêtres de 5 ms, voie la moins fidèle de chaque module) : ≥ 0,98 pour 29 modules, 0,95 à 0,98 pour 7, 0,90 à 0,95 pour 5, sous 0,90 pour 8. Les cas examinés sous 0,90 sont des artefacts de mesure : sur une note tenue, la corrélation ne mesure que du bruit, alors que niveaux et périodes sont identiques. Les autres n'ont pas tous été examinés.
- Écarts trouvés en route et réglés :
  - libopenmpt joue par défaut les passages inaccessibles comme des sous-morceaux (d'où `--subsong 0`) ;
  - libopenmpt arrondit la durée d'un tick à un nombre entier d'échantillons, d'où une légère dérive hors 125 BPM. Notre timing exact est plus proche de l'Amiga ;
  - deux corrections du replayer : 9xx au-delà de la fin d'un sample bouclé joue la boucle, et E9x re-déclenche aussi au tick 0 sur une ligne sans note.
- Outils de diagnostic dans `examples/` : `compare`, `isolate` (une voie seule), `trace` (positions), `periods` (période et volume tick par tick), `effects`, `odd_periods`.

### Phase 3 — L'agent compose seul (premier MVP)
- Couche de commandes avec undo/redo et journal, et serveur MCP `smpltrckr mcp` en mode sans TUI : l'agent travaille directement sur des fichiers.
- Générateur de samples (`sample_generate`) et import de WAV et d'AIFF, pour que l'agent ait de la matière sonore sans aide.
- Ressources MCP pour l'agent : référence des effets ProTracker, gammes et accords.
- Outils de mixage pour l'agent : `voice_mute`, `voice_solo`, `voice_volume`, et `render_wav` avec une option « une piste par voie ».
- **Jalon (= premier MVP) :** à partir d'une consigne courte (« un morceau chiptune de 30 secondes en la mineur »), un agent crée un `.mod` de A à Z et rend le WAV. Le fichier se lit dans pt2-clone ou OpenMPT. Les traces de la session sont conservées.

**Résultats (03/10/2026).** Jalon atteint techniquement. L'écoute reste à faire par JB.
- `src/editor.rs` : toute modification passe par l'éditeur, s'annule et se rétablit (500 niveaux), et laisse une trace dans le journal (heure, origine agent ou clavier). Le journal est enregistré à côté du `.mod` (`.journal.txt`).
- `src/samples.rs` : générateur de formes d'onde en boucle (sine, square, pulse, saw, triangle, noise) et de percussions (kick, snare, hihat). Import de WAV (8, 16, 24 ou 32 bits, flottant, boucle `smpl`) et d'AIFF ou AIFF-C, en mono 8 bits, sans rééchantillonnage, avec l'option « réduire de moitié ». La note qui rend la hauteur d'origine est indiquée.
- `src/mcp.rs` : 22 outils (morceau, ordre, patterns, samples, mixage, rendu WAV, annulation, journal), plus un aide-mémoire (`reference` : guide, effets, notes, gammes, accords).
- **Session de composition** (`sessions/2026-10-03-chiptune-la-mineur/`). Claude a reçu une seule consigne : « un morceau chiptune d'environ 30 secondes en la mineur ». Il a composé « La Mineur Chip » en 2 min 20 s (25 tours, 0,64 $) : 4 patterns, Am–F–C–G puis une cadence par mi majeur, arpèges 037/047, mélodie pulse avec vibrato, fin en fondu et F00. openmpt123 lit le fichier (30,72 s) et l'aller-retour est exact. Une seule erreur de notation (un effet écrit dans la colonne de la note), que l'agent a corrigée seul ; le message d'erreur propose maintenant directement la bonne écriture.

### Phase 4 — MVP de la TUI
- Éditeur de patterns (curseur, saisie de notes au clavier piano, numéro de sample, effets), liste d'ordre, liste de samples, lecture/arrêt, sauvegarde.
- Raccourcis simples inspirés de ProTracker, réunis dans une seule table. La disposition du clavier est reportée.
- **Un affichage par voie, dans sa colonne.** Sous chaque colonne du pattern, un VU-mètre simple et l'état de la voie (coupée, solo). Un **master** à droite.
- **Mixage au clavier :** coupure, solo et volume de chaque voie (une touche par voie, comme les touches F1 à F4 de ProTracker), pour faire des mix.
- **Contrôles :** sélection du sample courant, ouverture et enregistrement de fichiers (petit navigateur de fichiers en mode texte), chargement d'un sample WAV ou AIFF, réglages du sample (volume, finetune, boucle), tempo et vitesse.
- **Jalon :** JB ouvre un morceau composé par l'agent, l'écoute, le retouche au clavier et l'enregistre.

**Résultats (03/10/2026).** Interface en place, jalon à valider par JB au clavier.
- `smpltrckr edit [fichier.mod]` (`src/tui/`). Pattern centré sur le curseur ; un VU-mètre, le volume et l'état (coupée, SOLO) sous chaque voie ; master à droite. Liste d'ordre et samples dans des panneaux (F6, F7) ; cadre rouge en mode édition, comme dans ProTracker.
- Saisie au clavier piano (QWERTY physique, deux octaves, F1/F2 pour l'octave), sample en décimal et effet en hexadécimal dans leurs colonnes, insertion et suppression de lignes. Les notes sonnent pendant la saisie, même à l'arrêt.
- Lecture du morceau (Entrée) ou d'un pattern en boucle (Ctrl+P), édition possible pendant la lecture (le curseur suit). Coupure, solo et volume par voie (Alt+1…8, Alt+S, Alt+↑↓).
- Samples : volume, finetune, génération, chargement WAV/AIFF via un petit navigateur de fichiers, renommage, écoute. Fichiers : ouvrir, enregistrer, enregistrer sous (avec le journal), annuler et rétablir, confirmation avant de perdre des modifications.
- Tous les raccourcis sont dans une seule table (`src/tui/keys.rs`), affichée par « ? ».
- 56 tests, dont des tests d'écran (sans terminal). Scénario complet vérifié sur le vrai binaire, dans un terminal simulé, sur le morceau de l'agent : notes saisies, lecture, arrêt, enregistrement sous un autre nom, journal « clavier ».
- Limite connue : le clavier piano suit les touches d'un clavier QWERTY. Sur un clavier AZERTY, les notes ne tombent pas aux bonnes places (disposition reportée, cf. décisions).

### Phase 5 — Humain et agent en direct
- La TUI ouvre son socket. Le pont `smpltrckr mcp` s'y connecte quand une instance tourne, et reste en mode sans TUI sinon.
- Les modifications de l'agent s'affichent en direct, avec une marque visuelle et l'historique.
- **Jalon :** l'agent pose une base (rythme et basse) pendant que JB la retravaille à la main dans la même session.

**Résultats (03/10/2026).** Jalon atteint, à la main et à deux : JB trouve l'essai « vraiment concluant ».
- Première session à deux, sur « Duo en la mineur ». L'agent (cette session Claude, passée par `smpltrckr mcp`) a posé la batterie, la charleston et la basse (la mineur, fa, do, sol). JB a écrit la mélodie sur la voie 4, avec sa propre sinusoïde. À sa demande, l'agent a ajouté des glissés (310) sur la mélodie, renommé le sample (« Thérémine de bord »), puis écrit une contre-mélodie dans un pattern 01 qui remplace la charleston (4 voies obligent).
- Une seule boucle possède le morceau : celle de l'interface. `src/session.rs` regroupe le morceau, son fichier, l'état « non enregistré » et la position de l'utilisateur. L'agent n'y touche que par des travaux (jobs) exécutés par l'interface entre deux images.
- L'interface ouvre un socket local (`$XDG_RUNTIME_DIR/smpltrckr.sock`, ou `SMPLTRCKR_SOCKET`) et y fait tourner le serveur MCP. `smpltrckr mcp` sert de relais stdio vers ce socket quand une interface est ouverte, et retombe sinon sur le mode sans écran. `ping` dit à l'agent dans quel mode il est.
- À l'écran : les cellules écrites par l'agent sont teintées de vert pendant 20 s, la ligne d'état dit ce qu'il vient de faire, une pastille « agent » s'allume dans l'en-tête, et le journal (F8) montre qui a fait quoi. Le son suit en direct, même pendant la lecture.
- Côté agent : `song_info` donne la position de l'utilisateur. `song_new` et `song_load` sont refusés tant que l'utilisateur a des modifications non enregistrées. Tout reste en anglais, même avec une interface en français. Le guide a gagné une section « Live sessions ».
- Vérifié : interface lancée dans un terminal simulé, client MCP passant par `smpltrckr mcp`. Ses notes arrivent dans le morceau, son `song_new` est refusé, son enregistrement va dans le fichier de l'utilisateur, le socket disparaît à la fermeture. Le repli sans écran fonctionne aussi. 70 tests.
- Limite : `smpltrckr mcp` choisit son mode au lancement. Si l'agent démarre avant l'interface, il reste sans écran jusqu'à ce qu'on relance son serveur MCP (`/mcp` dans Claude Code).

### Phase 6 — Le look
- **Forme d'onde de chaque voie, dans sa colonne,** avec un effet de particules : les crêtes projettent des points (Braille ou demi-blocs) qui retombent et s'estompent. Master à droite, avec la forme d'onde du mixage final.
- VU-mètres en blocs de couleur, forme d'onde des samples en caractères.
- Couleurs reprises du thème Omarchy actif. À vérifier : où et sous quelle forme Omarchy expose les couleurs du thème courant.
- Écran d'accueil en ASCII art, et pourquoi pas un mode « démo » plein écran pendant la lecture (clin d'œil au rendu texte de VLC).

**Résultats (04/10/2026).** Jalon atteint : JB a validé le rendu à l'écran.
- **Couleurs** (`src/tui/theme.rs`) : l'interface utilise des rôles (texte, accent, notes, samples, effets, oscilloscopes, curseur, édition, agent…). Le thème Omarchy actif est lu dans `~/.local/state/omarchy/current/theme/colors.toml` et relu chaque seconde, ce qui suit un changement de thème à chaud. `--theme classic` garde les couleurs d'origine.
- **Particules** (`src/tui/particles.rs`) : à chaque attaque d'une voie, la crête de son oscilloscope lâche une gerbe de points qui montent, retombent sous la gravité et s'estompent. Quelques étincelles partent aussi tant que la note sonne.
- **Écran d'accueil** : le logo SMPLTRCKR en blocs, en dégradé de couleurs du thème, pendant 2,5 s ou jusqu'à la première touche.
- **Mode démo** (F9) : essayé puis retiré à la demande de JB, qui n'y voyait pas d'intérêt.
- Retouches demandées par JB : nom SMPLTRCKR en capitales et en dégradé dans l'en-tête, une ligne vide entre l'en-tête et le pattern, auteur (@jbheren) sur l'accueil et en bas à droite, liste des samples alignée sur la liste d'ordre.
- 76 tests. La capture texte de l'accueil est dans un test ignoré (`cargo test snapshot_show -- --ignored --nocapture`).

### Phase 7 — Le XM
- Lecture et écriture du `.xm`, avec un aller-retour à l'octet près comme pour le `.mod`.
- Instruments (enveloppes de volume et de panoramique, plusieurs samples par instrument), samples 16 bits, jusqu'à 32 voies, colonne de volume.
- Second replayer façon FT2, comparé à libopenmpt et à ft2-clone.
- Outils MCP et TUI étendus aux instruments.
- **Jalon :** l'agent compose un `.xm` qui se lit correctement dans ft2-clone ou MilkyTracker.

### Phase 8 — Plus tard (à trier)
- Disposition AZERTY, protocole clavier de kitty.
- Éditeur de samples minimal (couper, normaliser, boucle), rééchantillonnage à l'import.
- Outil `analyze` pour l'agent.
- Format IT.
- Lecture sur le Pirate Audio.

---

## 7. Risques

| Risque | Parade |
|---|---|
| Craquements audio (thread temps réel) | Pas d'allocation ni de verrou bloquant dans le callback audio. Le replayer reçoit des instantanés ou des messages par une file sans verrou. Test dès la phase 0. |
| Écarts de lecture subtils (effets, cas limites ProTracker) | Comparaison automatique avec libopenmpt sur un corpus, et pt2-clone comme référence d'écoute. |
| Écritures concurrentes humain / agent | Une seule boucle d'événements possède le modèle. Les commandes s'exécutent l'une après l'autre et chacune peut être annulée. |
| Samples : l'agent a besoin de matière sonore | Générateur de formes d'onde intégré dès le MVP, et un dossier de samples local. |

---

## 8. Questions ouvertes pour JB

*Tranchées le 02/10/2026 : rôle de l'agent, langage, clavier, fidélité Amiga, nombre de voies, samples, XM, nom, publication, mention de Claude dans les commits, modèle, enregistrement du mixage (voir « Décisions de JB » en tête du document).*

Plus aucune question ouverte pour l'instant.

---

## 9. Dépôt et commits

- Le dépôt Git reste local jusqu'à ce que JB demande de le publier. Il sera alors publié sur `github.com/jbheren/smpltrckr`.
- Les commits sont au nom de JB, sans ligne `Co-Authored-By: Claude` pour l'instant. La configuration Git globale du poste les signe déjà `jbheren <jb@lemaitrezor.com>`. Pour que GitHub relie les commits au compte, cette adresse doit être déclarée sur le compte `jbheren`.
- Pour publier, il faudra se connecter à GitHub sur le poste (`gh auth login`). Ce n'est pas encore fait.

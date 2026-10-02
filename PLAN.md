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

### Phase 1 — Le format `.mod`
- Parseur et écrivain `.mod`, et conversion vers/depuis la notation texte.
- Outils en ligne de commande : `smpltrckr dump fichier.mod` (texte) et `smpltrckr roundtrip fichier.mod`.
- Corpus de test (quelques dizaines de modules de The Mod Archive, variantes M.K., FLT4, 8CHN).
- **Jalon :** sur tout le corpus, un module chargé puis réenregistré est **identique à l'octet près**. *(C'est la condition de la note pour passer au statut de chantier.)*

### Phase 2 — Le replayer
- Replayer ProTracker, rendu WAV hors ligne (`smpltrckr render`).
- Tests de comparaison avec libopenmpt (ou pt2-clone) sur le corpus.
- **Jalon :** les rendus sont fidèles à l'oreille, et les écarts mesurés restent sous un seuil sur la plupart des modules du corpus.

### Phase 3 — L'agent compose seul (premier MVP)
- Couche de commandes avec undo/redo et journal, et serveur MCP `smpltrckr mcp` en mode sans TUI : l'agent travaille directement sur des fichiers.
- Générateur de samples (`sample_generate`) et import de WAV et d'AIFF, pour que l'agent ait de la matière sonore sans aide.
- Ressources MCP pour l'agent : référence des effets ProTracker, gammes et accords.
- **Jalon (= premier MVP) :** à partir d'une consigne courte (« un morceau chiptune de 30 secondes en la mineur »), un agent crée un `.mod` de A à Z et rend le WAV. Le fichier se lit dans pt2-clone ou OpenMPT. Les traces de la session sont conservées.

### Phase 4 — MVP de la TUI
- Éditeur de patterns (curseur, saisie de notes au clavier piano, numéro de sample, effets), liste d'ordre, liste de samples, lecture/arrêt, sauvegarde.
- Raccourcis simples inspirés de ProTracker, réunis dans une seule table. La disposition du clavier est reportée.
- **Jalon :** JB ouvre un morceau composé par l'agent, l'écoute, le retouche au clavier et l'enregistre.

### Phase 5 — Humain et agent en direct
- La TUI ouvre son socket. Le pont `smpltrckr mcp` s'y connecte quand une instance tourne, et reste en mode sans TUI sinon.
- Les modifications de l'agent s'affichent en direct, avec une marque visuelle et l'historique.
- **Jalon :** l'agent pose une base (rythme et basse) pendant que JB la retravaille à la main dans la même session.

### Phase 6 — Le look
- Oscilloscopes par voie (Braille ou demi-blocs), VU-mètres en blocs de couleur, forme d'onde des samples en caractères.
- Couleurs reprises du thème Omarchy actif. À vérifier : où et sous quelle forme Omarchy expose les couleurs du thème courant.
- Écran d'accueil en ASCII art, et pourquoi pas un mode « démo » plein écran pendant la lecture (clin d'œil au rendu texte de VLC).

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

*Tranchées le 02/10/2026 : rôle de l'agent, langage, clavier, fidélité Amiga, nombre de voies, samples, XM, nom, publication, mention de Claude dans les commits, modèle (voir « Décisions de JB » en tête du document).*

Plus aucune question ouverte pour l'instant.

---

## 9. Dépôt et commits

- Le dépôt Git reste local jusqu'à ce que JB demande de le publier. Il sera alors publié sur `github.com/jbheren/smpltrckr`.
- Les commits sont au nom de JB, sans ligne `Co-Authored-By: Claude` pour l'instant. La configuration Git globale du poste les signe déjà `jbheren <jb@lemaitrezor.com>`. Pour que GitHub relie les commits au compte, cette adresse doit être déclarée sur le compte `jbheren`.
- Pour publier, il faudra se connecter à GitHub sur le poste (`gh auth login`). Ce n'est pas encore fait.

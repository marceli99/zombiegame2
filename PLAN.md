# Plan poprawek — zombiegame2

> **ARCHIWUM (2026-08-27):** plan zrealizowany w całości (ETAP 1–6 ✅) i
> zamknięty. Dokument zachowany wyłącznie jako historyczny zapis — nie jest to
> aktualna roadmapa projektu.

> Utworzony: 2026-05-07
> Źródło: pełen audyt kodu (combat / net / map / UI / code quality) + weryfikacja własna.

## Zasada podziału

- **Każdy etap = jedna spójna seria commitów** (najlepiej osobny PR).
- Etapy 1-2 dotykają gameplay i sieci → najpierw stabilizujemy rozgrywkę.
- Etapy 3-4 to UX i feedback dla gracza.
- Etap 5 to porządki w kodzie bez wpływu na działanie gry.
- Etap 6 to większa refaktoryzacja architektury (osobno, bo ryzykowny diff).

Numer w nawiasie kwadratowym przy zadaniu = priorytet wewnątrz etapu (1 = najpierw).

---

## ETAP 1 — Krytyczne bugfixy gameplay ✅

Cel: usunąć błędy logiki, które każda rozgrywka odczuwa.

- [x] **[1] `zombie.rs:1547-1556` — Burning zombie**
  Zachowane contact damage (rate-limited przez `invuln_timer = 0.5s`), ale `BurnEffect` wstawiany tylko jeśli go jeszcze nie ma — eliminuje refresh loop, który nadpisywał `remaining` i `accumulated` co tick. Burn naturalnie wygasa po 10 s, nawet jeśli gracz wciąż jest w kontakcie.

- [x] **[2] `weapon.rs:1099` — throwables**
  Wyrównane do `gen_range(1..=3)` we wszystkich miejscach.

- [x] **[3] `weapon.rs:1234` — `money_mult`**
  Weryfikacja: audit się mylił. `player.rs:919` resetuje `money_mult = 1` po wygaśnięciu timera. `.max()` to świadomy wybór (player-friendly: słabszy pickup nie downgrade'uje, ale przedłuża timer). Dodano komentarz wyjaśniający intencję.

- [x] **[4] `wave.rs:123` — wave scaling**
  Zmienione na `players.iter().count()`. `dead_players` jest drenowany przy wave-clear, więc dodatek był redundantny.

- [x] **[5] `zombie.rs:1617` — game over after BurnEffect**
  Logika sama w sobie była poprawna (audit się mylił), ale **znaleziono głębszy bug**: burn-killed gracze nie byli dodawani do `DeadPlayers` ani nie wysyłali `PlayerDiedEvent`, więc nigdy nie respawnowali się i nie mieli zwłok/animacji śmierci. Dodano pełen flow śmierci jak w `player.rs::player_damage`. Zmieniona nazwa `dead_ids` → `newly_dead` dla jasności.

- [x] **[6] `weapon.rs:908` — pickup swap delay**
  `fire_cooldown = 0.15` (jak przy ręcznym przełączeniu slotu) zamiast `0.0`. Zapobiega instant-fire na klatce pickupu.

**Build:** `cargo check` czysty, brak nowych warningów.

**Smoke test do wykonania ręcznie:** SP, host LAN + 1 klient — fala 1 do 5, pickup throwables/2x/3x, kontakt z Burning (burn powinien naturalnie wygasać), umrzeć od burn DoT (powinien wyspawnować się trup + respawn na końcu fali), pickup broni (brak natychmiastowego strzału).

---

## ETAP 2 — Stabilizacja sieci ✅

Cel: wyeliminować desync, race condition i scenariusze padaki.

- [x] **[1] `lobby.rs:150,161` — mutex unwrap**
  Zastąpione użyciem helpera `broadcast()` z `net.rs`, który już ma poison recovery. Załatwia E2.1 i E2.5 jednocześnie.

- [x] **[2] `sync.rs:140` — mid-game join**
  Listener teraz odrzuca podczas `Playing` z `ServerMsg::GameInProgress`. Klient widzi `ClientInEvent::GameInProgress` i wraca do menu jak przy `FullLobby`. Host ustawia/zeruje `Arc<AtomicBool> in_game` na transition. Bumped `PROTOCOL_VERSION = 6`.

- [x] **[3] `net.rs:478-479` — race + u8 wrap**
  Race jest niemożliwy (accept jest sekwencyjny w jednym wątku). Wrap u8 → 0 (kolizja z hostem) **był** prawdziwy. Zamiast wielkiego refaktoru u8 → u32, zmieniony na alokację pierwszego wolnego ID przez skan `senders` HashMap. Brak wraparound, brak kolizji ze stale clientami.

- [x] **[4] `sync.rs:670` — stale input**
  Audit się mylił (despawnowany player nie pojawia się w `Query<Player>`, więc `unwrap_or_default()` nie jest stosowany do martwego). **Znaleziony głębszy bug**: encja gracza po disconnect nigdy nie była despawnowana po stronie hosta — była wiecznie w snapshotach i blokowała kolizje. Naprawione.

- [x] **[5] `lobby.rs` — broadcast lock**
  Zweryfikowane: `mpsc::channel()` jest unbounded, więc lock-contention jest teoretyczny. Już rozwiązane przez użycie helpera `broadcast()`.

- [x] **[6] `sync.rs:815` — boss spawn**
  WIP użytkownika już to rozwiązał — `boss_spawn_evw.send()` jest tylko w branchu `None =>` (nowo pojawiający się Giant). Bez zmian.

- [x] **[7] `chat.rs:295` — local echo**
  Klient teraz lokalnie echuje wiadomość *tylko jeśli send fails* (np. writer thread padł). Normalny flow nadal idzie przez serwer (uniknij duplikatu).

- [x] **[8] `sync.rs:197` — stale 30Hz comment**
  Wyczyszczone obok dwa stale komentarze (linia 33-43 i linia 197).

**Build:** `cargo check` czysty po wszystkich zmianach. `PROTOCOL_VERSION` bumped 5 → 6.

**Smoke test do wykonania ręcznie:**
- dołączanie/rozłączanie w lobby (3+ graczy, ID-y się dobrze przypisują)
- klient próbuje join podczas trwającej fali → wraca do menu z disconnect
- klient rozłącza się w trakcie fali → host kontynuuje, jego encja znika
- czat: wpisać, host odbiera, klient widzi swoją wiadomość raz

---

## ETAP 3 — UX core (rzeczy, które gracz natychmiast czuje) ✅

Cel: feedback, kontrola, zgodność oczekiwań.

- [x] **[1] `chat.rs` — pauza vs czat**
  Czat zamknięty przy `OnEnter(PauseState::Paused)` (clear bufora) + `chat_input_system` runs only `in_state(PauseState::Running)`. Q/M w pauzie nie idą już do czatu.

- [x] **[2] Settings hot-apply**
  Weryfikacja: audit się mylił. `apply_graphics_settings` działa per `Res::is_changed()`, mutacje cycle/toggle wymuszają fire `is_changed` następnym frame'em → window aktualizowany. Bez zmian. Linia `menu.rs:443` z audytu to nawet nie był settings handler.

- [x] **[3] Lobby start countdown**
  Dodany `LobbyCountdown` resource. Host: pierwszy Enter → 3 s countdown + `ServerMsg::CountdownStart`, drugi Enter/Esc → cancel + `CountdownCancel`. Klient: mirror countdown z eventów. UI pokazuje `STARTING IN N...`. `PROTOCOL_VERSION` 6 → 7.

- [x] **[4] Speedrunner gameplay clock**
  Nowy `GameplayClock` resource, inkrementowany przez `tick_gameplay_clock` z `run_if(gameplay_active)`. `track_kills` i Speedrunner check używają teraz `clock.elapsed_seconds`. Pauza i menu między falami nie liczą się.

- [x] **[5] Impact sparks na explodables**
  Spawn `ImpactSparks` przed despawnem w `bullet_collision`, dla non-rocket non-flame kul trafiających w wraki/beczki. Bez zmiany dla rakiet (i tak mają explosion FX) i flame (dissipation visual).

- [x] **[6] Reset to Defaults**
  Nowy wiersz w settings menu (`SETTINGS_ROW_RESET = 6`, `SETTINGS_ROW_BACK = 7`). Enter na "RESET DEFAULTS" → `*settings = GraphicsSettings::default()` → następny frame `is_changed` → apply + save automatycznie.

**Build:** `cargo check` czysty bez warningów. Diff: 11 plików, +472/-99.

**Smoke test do wykonania ręcznie:**
- Pauza z otwartym czatem → buffer zostaje wyczyszczony, Q nie wpisuje się
- Settings: zmiana rozdzielczości / vsync → natychmiastowy efekt w oknie
- Settings: Reset Defaults → wraca do borderless 1280x720 vsync ON
- Lobby: Enter → 3-2-1 countdown widoczny u hosta i klienta
- Lobby: Enter (countdown trwa) → cancel
- Strzelanie do auta z pistoletu → iskry na każdym hicie
- Speedrunner po wave 5 — pauza między falami nie liczy się

---

## ETAP 4 — UX polish ✅

Cel: HUD, klarność, brak overflow.

- [x] **[1] HUD ammo dla wszystkich slotów**
  Nowy komponent `SlotAmmoText` per slot. Mała nakładka tekstowa w prawym dolnym rogu każdego slot iconu pokazuje liczbę nabojów (lub `∞` dla broni z infinite ammo, `xN` dla throwables).

- [x] **[2] Lobby join/leave toast**
  Nowy `LobbyToast` resource + `LobbyToastText` UI. `track_lobby_changes` diffuje `lobby_players` względem ostatniej klatki, wykrywa joins/leaves (host + klient widzą lokalnie), nicknames lookup → toast `"NAME JOINED"` / `"NAME LEFT"` na 2.5 s z fade.

- [x] **[3] HUD nick truncation**
  `Overflow::clip_x()` dodany na container nicku w player list. NICKNAME_MAX_LEN=10 chars, więc edge case rzadki, ale teraz bezpieczny.

- [x] **[4] Score overflow**
  Funkcja `format_compact_score(u32)` zamiast raw format. `<10K` raw, `<1M` `12.3K`, w innym wypadku `9.9M`. Wave overflow zostawiony — `WAVE 100` wpada w istniejący box.

- [x] **[5] Menu ESC behavior**
  Esc w menu nie robi już `process::exit(0)`. Zamiast tego ustawia kursor na "WYJSCIE" (index 6) — drugi Esc/Enter wyjdzie. Kursor scrolluje wizualnie tam, gdzie wyjście było zamierzonym efektem.

- [x] **[6] Pauza opacity SP/MP**
  Zostawione: SP 0.7, MP 0.5. Dodany komentarz wyjaśniający, że MP nie zatrzymuje świata, więc jaśniejsze tło pozwala czytać akcję za pauzą — projektowo zamierzone.

- ~~[7] T-key conflict~~ — zignorowane (no rebinding zaplanowany; opisowy komentarz wystarczy w docs jeśli kiedykolwiek dodamy rebind).

**Build:** `cargo check` czysty bez warningów. Diff: 13 plików, +685/-105.

**Smoke test do wykonania ręcznie:**
- Slot icons: liczba ammo widoczna przy każdym slocie, `∞` dla broni z infinite ammo, `xN` dla throwables
- Long nickname (max 10) → mieści się w polu HP bar
- Score 1_500_000 → wyświetla `$1.5M`
- ESC w głównym menu → kursor leci na WYJSCIE, drugi ESC wyjdzie
- Lobby: drugi gracz dołącza/wychodzi → 2.5 s zielony toast

---

## ETAP 5 — Code quality / dead code ✅

Cel: zmniejszyć powierzchnię kodu, włączyć ostrzeżenia kompilatora.

- [x] **[1] Logowanie uszkodzonych save'ów**
  `achievements.rs::AchievementSave::load` i `settings.rs::load_settings` — rozróżnia "nie ma pliku" (cicho) vs "parse error" (warn + zapis `.json.bak`). Uszkodzony save odzyskasz teraz manualnie.

- [x] **[2] Cleanup dead code**
  Skasowane: `ScreenVignette`, `FilmGrain`, `PostprocessAssets`, `setup_postprocess_assets`, `update_film_grain`, `build_vignette_image`, `build_grain_image` (postprocess overlays nigdy nie były włączone). Skasowane: `ThrowableKind::from_u8`, `ThrowableKind::fuse_time` (unused). Skasowane: `unlock_nav_rows` (planowana feature, której nie zrealizowano).

- [x] **[3] `ui.rs` `bumped` variable**
  Usunięty (był no-op po `state.punch_time = 0.0` które już robi to co trzeba). Dodany komentarz wyjaśniający logikę.

- [x] **[4] `max_money_mult` consolidation**
  Jeden generic helper `max_money_mult<'a>(impl Iterator<Item = &'a Player>)` zamiast dwóch wariantów query-specific. Call sites: `players.iter()` lub `players.iter().map(|(_, p)| p)`.

- [x] **[5] `world_to_tile` dedup**
  `weapon.rs::segment_for_world_x` używa teraz `crate::map::world_to_tile()` zamiast inline'owej formuły.

- [x] **[6] Magic numbers → const**
  `ui.rs`: `COMBO_PUNCH_DURATION`, `COMBO_PUNCH_OVERSHOOT`. `weapon.rs`: `PICKUP_JITTER_RADIUS`, `PICKUP_FIND_ATTEMPTS`, `PICKUP_SPAWN_EXCLUSION`, `PICKUP_FIT_RADIUS`.

- ~~[7] Split `update_combo` / `update_player_list`~~ — pominięte. Funkcje są długie ale czytelne i mają jasny pojedynczy cel; podział byłby pre-mature abstraction.

**Build:** `cargo check` czysty, brak warningów. Diff narastająco: 15 plików, +782/-280.

---

## ETAP 6 — Refaktoryzacja architektury ✅

Cel: posprzątać po pivocie mapy + zmniejszyć `map.rs`. Wykonany jako jeden duży etap.

### 6a. HOUSE_FURNITURE (zweryfikowane non-issue) ✅

**Wynik audytu:** funkcja `furniture_for_floor` w `map.rs:942` od dawna ma pełen layout dla `BuildingType::House` (Bed, Wardrobe, Toilet, Bathtub, Couch, TV, Bookshelf, Fridge, Stove, DiningTable, DiningChair). Audit wskazał starą empty `HOUSE_FURNITURE` *compat shimę*, ale faktyczny system mebli używa `BuildingType` + `FurnKind` i działa. Domy NIE są puste. Brak akcji.

### 6b. Cleanup legacy enums i shimów ✅

- [x] Skasowane warianty `BuildingKind` (Sheriff, Pharmacy, Diner, GeneralStore, Saloon, Church, GasStation) i cała legacy enum sekcja (~140 linii) z `map_data.rs`.
- [x] Skasowane warianty `StreetDecorKind` z `map_data.rs`.
- [x] Skasowane: `FLOOR_*`, `ZONE_*`, `BARRIER_*`, `N_FLOORS`, `FLOOR_Y_CENTER`, `FLOOR_NAMES`, `ZONE_TO_FLOOR`, `ZONE0..3_ROW_*`, `BARRIER_NORTH_Y/SOUTH_Y/UNDERGROUND_Y` z `map.rs`.
- [x] Skasowane: `ELEVATOR_HALF`, `ELEVATORS`, `ShopKind`, `ShopSpec`, `DoorSide`, `MarkerKind`, `ElevatorSpec`, `SHOPS`, `SHOP_WALL_THICK`, `SHOP_DOOR_WIDTH`, `shop_back_room_pos`, `shop_wall_rects` z `map.rs`.
- [x] Skasowany cały `src/elevator.rs` (pusty plugin) + wpis `mod elevator` i `ElevatorPlugin` z `main.rs`.
- [x] **Zachowane**: `src/zones.rs` (`ZoneState` używany przez `achievements.rs::Explorer` i `zombie.rs`).
- [x] **Skasowany (2026-09-06)**: `src/underground.rs` (metro / manhole / teleport) razem z hookami: clamp kamery w `lib.rs`, `UNDERGROUND_EXTENT_Y`/`world_min_y` w `map_obstacles.rs` i `bullet.rs`, dwustrefowy `clamp_player_y` w `player.rs`, podziemny spawn w `zombie.rs`.

### 6c. Split `map.rs` — pominięty ⚠️

Po analizie: `build_*_image` funkcje są **przeplecione** z systemami Bevy w map.rs (28 funkcji rozproszonych przez 1822-4914), nie ma ciągłego bloku do przeniesienia. Każdy fragment to osobny edit + ryzyko zepsucia importów. Wartość niska względem ryzyka — `map.rs` nadal działa poprawnie (5275 linii). Pomijamy. Można wrócić w przyszłości jeśli stanie się problemem.

### 6 final — Cleanup `#![allow(dead_code)]` ✅

- [x] Zdjęte `#![allow(dead_code)]` z `map.rs:1` i `map_data.rs:14`.
- [x] Naprawione warningi punktowo:
  - `FurnKind` enum: `#[allow(dead_code)]` z komentarzem (8 wariantów rezerwowych z pełnymi spritami)
  - `Segment` struct: `#[allow(dead_code)]` (name, difficulty fields used in tooling)
  - `BuildingType` enum: `#[allow(dead_code)]` (Civic, Market reserved for future bake)
  - `PropKind` enum: `#[allow(dead_code)]` (Car, Wreck reserved)
  - `GateKind` enum: `#[allow(dead_code)]` (Tunnel reserved)
  - `Gate.label`: `#[allow(dead_code)]` field
  - `world_consts::ZOMBIE_SPAWNS`: `#[allow(dead_code)]` const
  - Skasowany `ensure_walls_built()` z map.rs (no callers)
  - Skasowany `pub use ZOMBIE_SPAWNS` re-export z map_data.rs

**Build:** `cargo check` **zero warningów**. Diff narastający: 19 plików, +825/-535.

`map.rs`: 5355 → 5275 linii (przy okazji bugfixów też się zmienił).
`map_data.rs`: 362 → 238 linii.
Skasowany: `elevator.rs` (-11 linii).

---

## Kolejność wykonania

```
Etap 1 (gameplay bugs)
   └─→ Etap 2 (network stability)
          └─→ Etap 3 (UX core)
                 └─→ Etap 4 (UX polish)
                        └─→ Etap 5 (code quality)
                               └─→ Etap 6 (architektura)
```

Etapy 1-5 niezależne strukturalnie — można je commitować oddzielnie. Etap 6 rób na końcu, na czystej mainie.

## Decyzje do podjęcia przed startem

1. **Etap 6a**: meble w chatach — wypełniać czy kasować?
2. **Etap 5.2**: postprocess (vignette + film grain) — przywracamy czy kasujemy?
3. **Etap 4.6**: ESC w menu — całkiem nic, czy focus na "WYJSCIE"?

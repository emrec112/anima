//! Every number behind how the characters behave, in one table: the daemon
//! reads them, the settings window builds its pages from them, and they live
//! in `[character.tuning]` in the config (only those changed from default).
//! They apply at once, without restarting anything.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicPtr, Ordering};

/// One setting, as the settings window shows it.
pub struct Field {
    pub tab: &'static str,
    pub group: &'static str,
    pub key: &'static str,
    pub label: &'static str,
    pub default: f32,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub digits: u32,
}

macro_rules! tuning {
    ($($tab:literal, $group:literal { $($key:ident: $label:literal = $def:expr, $min:expr, $max:expr, $step:expr, $digits:expr;)* })*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct Tuning { $($(pub $key: f32,)*)* }

        impl Tuning {
            pub const DEFAULT: Tuning = Tuning { $($($key: $def,)*)* };
        }

        pub const FIELDS: &[Field] = &[
            $($(Field { tab: $tab, group: $group, key: stringify!($key), label: $label, default: $def, min: $min, max: $max, step: $step, digits: $digits },)*)*
        ];
    };
}

tuning! {
    "Games", "Fishing" {
        fish_min: "Fewest players" = 2.0, 1.0, 12.0, 1.0, 0;
        fish_time: "Longest time (s)" = 300.0, 10.0, 7200.0, 5.0, 0;
        fish_bored: "Bored enough to fish alone after (s)" = 240.0, 5.0, 1800.0, 5.0, 0;
        fish_bored_every: "Then goes fishing within about (s)" = 180.0, 1.0, 1800.0, 5.0, 0;
        fish_glee: "Anglers have a happy moment while waiting (per s)" = 0.03, 0.0, 1.0, 0.01, 2;
        fish_join: "Passers-by join in (per s)" = 0.03, 0.0, 2.0, 0.01, 2;
        fish_most: "Most fishing together" = 5.0, 1.0, 12.0, 1.0, 0;
        fish_real: "A good catch is a real fish (share)" = 0.75, 0.0, 1.0, 0.05, 2;
        fish_creature: "Of those, a rare sea creature (share)" = 0.15, 0.0, 1.0, 0.05, 2;
        fish_find: "Something to wear on the line (share of catches)" = 0.12, 0.0, 1.0, 0.01, 2;
        fish_boom: "Confetti goes off with a bang" = 0.25, 0.0, 1.0, 0.05, 2;
        fish_boom_reach: "Explosion reach (widths)" = 3.0, 0.5, 15.0, 0.5, 1;
        fish_beast: "Something comes up to scare them (per s)" = 0.004, 0.0, 2.0, 0.01, 2;
        fish_beast_cool: "Quiet after a scare (s)" = 120.0, 0.0, 300.0, 5.0, 0;
        fish_beast_reach: "Scares anglers within (widths)" = 5.0, 1.0, 30.0, 0.5, 1;
        fish_reel: "Reeling in takes (s)" = 2.5, 0.3, 15.0, 0.1, 1;
        fish_stay_min: "Stays interested, at least (s)" = 45.0, 10.0, 3600.0, 10.0, 0;
        fish_stay_rand: "Random extra interest (s)" = 60.0, 0.0, 3600.0, 10.0, 0;
        fish_catch_keeps: "A good catch keeps it going (s)" = 5.0, 0.0, 600.0, 5.0, 0;
        fish_lost_cost: "A lost fish or junk costs (s)" = 15.0, 0.0, 300.0, 5.0, 0;
        fish_again: "Won't fish again for (s)" = 300.0, 0.0, 3600.0, 10.0, 0;
        fish_throw_far: "Thrown this far outside the window, gives up (widths)" = 6.0, 1.0, 40.0, 0.5, 1;
        fish_wait_min: "Waiting for a bite, at least (s)" = 12.0, 0.5, 60.0, 0.5, 1;
        fish_wait_rand: "Random extra wait (s)" = 28.0, 0.0, 180.0, 0.5, 1;
    }
    "Games", "Snowball fight" {
        snow_min: "Fewest players (asked for by hand)" = 2.0, 2.0, 12.0, 1.0, 0;
        snow_start_every: "While it snows, someone starts one within about (s)" = 75.0, 5.0, 3600.0, 5.0, 0;
        snow_join: "Passers-by join in (per s)" = 0.12, 0.0, 2.0, 0.01, 2;
        snow_stay_min: "Keeps at it, at least (s)" = 45.0, 5.0, 600.0, 5.0, 0;
        snow_stay_rand: "Random extra (s)" = 75.0, 0.0, 600.0, 5.0, 0;
        snow_again: "Won't start another for (s)" = 150.0, 0.0, 3600.0, 10.0, 0;
        snow_longest: "Winds down after (s)" = 360.0, 30.0, 3600.0, 10.0, 0;
        snow_dodge: "Hops out of the way of a ball (share)" = 0.3, 0.0, 1.0, 0.05, 2;
        snow_roll_rate: "Someone starts a giant snowball (per s)" = 0.02, 0.0, 1.0, 0.01, 2;
        snow_roll_min: "Players needed for that" = 2.0, 2.0, 30.0, 1.0, 0;
        snow_help: "Someone from its side comes to help roll it (per s)" = 0.15, 0.0, 2.0, 0.01, 2;
        snow_roll_speed: "Giant ball rolls up at (px/s)" = 70.0, 5.0, 300.0, 5.0, 0;
        snow_grow: "Giant ball grows (px wider per px rolled)" = 0.3, 0.01, 2.0, 0.01, 2;
        snow_big: "Giant ball grows to (heights)" = 1.8, 0.3, 4.0, 0.05, 2;
        snow_resist: "The other side runs over to push it back (share)" = 0.5, 0.0, 1.0, 0.05, 2;
        snow_giant_speed: "Sent off, it rolls at (px/s)" = 380.0, 50.0, 2000.0, 10.0, 0;
        snow_flat: "Flattened for (s)" = 1.4, 0.2, 10.0, 0.1, 1;
    }
    "Games", "Snowman" {
        snowman_every: "While it snows, someone starts one within about (s)" = 200.0, 5.0, 3600.0, 5.0, 0;
        snowman_most: "Most snowmen on a workspace" = 3.0, 1.0, 20.0, 1.0, 0;
        snowman_help: "Someone comes to help (per s)" = 0.06, 0.0, 2.0, 0.01, 2;
        snowman_size: "Bottom ball (heights)" = 0.8, 0.2, 3.0, 0.05, 2;
        snowman_roll_speed: "Rolled up at (px/s)" = 40.0, 5.0, 300.0, 5.0, 0;
        snowman_stay: "Snowman stays (s)" = 900.0, 10.0, 7200.0, 30.0, 0;
        snowman_dress_every: "A passer-by puts something on one within about (s)" = 90.0, 5.0, 3600.0, 5.0, 0;
    }
    "Roaming", "Reading" {
        read_bored: "Idle before taking out a book (s)" = 30.0, 0.0, 1800.0, 5.0, 0;
        read_every: "Then reads within about (s, book lovers sooner)" = 300.0, 5.0, 3600.0, 5.0, 0;
        read_min: "Reads at least (s)" = 30.0, 5.0, 1800.0, 5.0, 0;
        read_max: "Reads at most (s)" = 120.0, 5.0, 3600.0, 5.0, 0;
        read_join: "A passer-by comes to read beside them (per s)" = 0.04, 0.0, 2.0, 0.01, 2;
        read_most: "Most reading together" = 3.0, 1.0, 12.0, 1.0, 0;
        read_laugh_every: "Laughs at the book about every (s)" = 18.0, 2.0, 600.0, 1.0, 0;
        read_laugh_len: "A laugh lasts (s)" = 1.8, 0.3, 10.0, 0.1, 1;
        read_sync: "The one beside laughs along (share)" = 0.45, 0.0, 1.0, 0.05, 2;
        read_again: "Won't read again for (s)" = 420.0, 0.0, 7200.0, 10.0, 0;
    }
    "Games", "Hide and seek" {
        hide_min: "Fewest players" = 3.0, 2.0, 12.0, 1.0, 0;
        hide_time: "Longest time (s)" = 90.0, 15.0, 600.0, 5.0, 0;
        hide_count: "Counts for (s)" = 6.0, 1.0, 60.0, 0.5, 1;
        hide_seek_speed: "Seeker walks at" = 0.13, 0.02, 1.0, 0.01, 2;
        hide_miss: "Seeker looks right past them (share)" = 0.15, 0.0, 1.0, 0.05, 2;
        hide_dash: "Makes a dash for the start (per s)" = 0.06, 0.0, 2.0, 0.01, 2;
    }
    "Games", "Getting better" {
        skill_rate: "Learning per game played" = 0.1, 0.0, 1.0, 0.01, 2;
        skill_teach: "Learning from a better friend" = 0.15, 0.0, 1.0, 0.01, 2;
        train_energy: "Energy to go and train alone" = 0.6, 0.0, 1.0, 0.05, 2;
        train_bored: "Idle before training (s)" = 40.0, 0.0, 1800.0, 5.0, 0;
        train_every: "Goes to train within about (s)" = 120.0, 1.0, 3600.0, 5.0, 0;
        train_reps: "Moves per session" = 6.0, 1.0, 30.0, 1.0, 0;
        train_cost: "Energy a session costs" = 0.3, 0.0, 1.0, 0.05, 2;
        train_again: "Rest before training again (s)" = 600.0, 0.0, 7200.0, 30.0, 0;
        mentor_chance: "A better one comes to teach" = 0.6, 0.0, 1.0, 0.05, 2;
        mentor_gap: "Teacher at least this much better" = 0.25, 0.0, 1.0, 0.05, 2;
        mentor_boost: "Learning from a teacher" = 0.5, 0.0, 2.0, 0.05, 2;
        skill_volley: "Beginners miss the ball" = 0.3, 0.0, 1.0, 0.05, 2;
    }
    "Roaming", "Liveliness" {
        land_laugh: "Laughing after being thrown (s)" = 0.5, 0.0, 5.0, 0.1, 1;
        laugh_time: "Laughing it off after a tumble (s)" = 1.6, 0.0, 10.0, 0.1, 1;
        laugh_spread: "Friends nearby laugh along" = 0.6, 0.0, 1.0, 0.05, 2;
        look_around: "Looking around while standing (per s)" = 0.12, 0.0, 2.0, 0.01, 2;
        greet_cool: "Between waves at the same friend (s)" = 90.0, 5.0, 900.0, 5.0, 0;
        pat_distract: "Happy moment when petted mid-game (s)" = 0.7, 0.2, 10.0, 0.1, 1;
    }
    "Social", "Parting" {
        part_stay: "Stay where they are" = 0.25, 0.0, 1.0, 0.05, 2;
        part_cross: "Cross to the far side" = 0.2, 0.0, 1.0, 0.05, 2;
        part_other: "Head to another group" = 0.15, 0.0, 1.0, 0.05, 2;
        part_buddy: "Leave with a neighbour" = 0.25, 0.0, 1.0, 0.05, 2;
        part_stagger: "Spread of leaving (s)" = 3.0, 0.0, 15.0, 0.5, 1;
    }
    "Games", "Cheating (the cute kind)" {
        tug_caught: "Tug cheater gets noticed" = 0.5, 0.0, 1.0, 0.05, 2;
        carrot_thief_chance: "Someone snatches and eats the carrot" = 0.2, 0.0, 1.0, 0.05, 2;
        carrot_eat: "Eating the carrot (s)" = 4.0, 0.5, 20.0, 0.5, 1;
        carrot_pass_chance: "Slip the carrot to a friend" = 0.35, 0.0, 1.0, 0.05, 2;
        long_cheat_chance: "Long jump: nudged further" = 0.25, 0.0, 1.0, 0.05, 2;
        long_caught: "Nudge gets noticed" = 0.5, 0.0, 1.0, 0.05, 2;
        long_push_px: "Nudge distance (px)" = 160.0, 10.0, 1000.0, 10.0, 0;
        long_push_speed: "Nudge speed (px/s)" = 220.0, 20.0, 2000.0, 10.0, 0;
        bribe_chance: "Bribe before a game" = 0.12, 0.0, 1.0, 0.02, 2;
        long_bribe_bonus: "Bribe bonus to the jump" = 0.4, 0.0, 3.0, 0.05, 2;
    }
    "Games", "Concert after a race" {
        concert_chance: "Chance of a concert" = 0.8, 0.0, 1.0, 0.05, 2;
        race_travel: "Come from other workspaces to race" = 0.85, 0.0, 1.0, 0.05, 2;
        concert_time: "Concert length (s)" = 20.0, 3.0, 120.0, 1.0, 0;
    }
    "Roaming", "Visiting fullscreen apps" {
        fs_first: "Until the first visit (s)" = 60.0, 1.0, 1800.0, 5.0, 0;
        fs_every: "Time between visits (s)" = 240.0, 5.0, 3600.0, 5.0, 0;
        fs_chance: "Chance of a visit" = 0.7, 0.0, 1.0, 0.05, 2;
        fs_max: "Most visitors" = 3.0, 1.0, 12.0, 1.0, 0;
        fs_fish: "A visitor sits down to fish" = 0.3, 0.0, 1.0, 0.05, 2;
        fs_stay_min: "Shortest stay (s)" = 25.0, 1.0, 600.0, 5.0, 0;
        fs_stay_rand: "Random extra stay (s)" = 35.0, 0.0, 600.0, 5.0, 0;
    }
    "Roaming", "Moving between workspaces" {
        ws_every: "Moving interval per character (s)" = 240.0, 10.0, 3600.0, 10.0, 0;
        ws_floor: "Smallest share per workspace" = 0.05, 0.0, 0.5, 0.05, 2;
        ws_use_weight: "Use vs window count (1 = use)" = 0.5, 0.0, 1.0, 0.05, 2;
        ws_use_half: "Use halves every (s)" = 900.0, 30.0, 14400.0, 30.0, 0;
        ws_stay: "Stays at least (s)" = 120.0, 0.0, 3600.0, 10.0, 0;
        ws_gap: "Quiet after someone leaves (s)" = 45.0, 0.0, 1800.0, 5.0, 0;
        ws_friends: "Friends who may come along, at most" = 0.0, 0.0, 6.0, 1.0, 0;
        ws_max: "On the way at once" = 2.0, 1.0, 12.0, 1.0, 0;
        ws_speed: "Travel speed" = 2.5, 0.2, 20.0, 0.1, 1;
    }
    "Roaming", "Park" {
        park_mode: "With Anima Park open: 0 desktop, 1 only park, 2 park and out now and then, 3 the desktop, visiting park" = 3.0, 0.0, 3.0, 1.0, 0;
        park_play: "Someone free in the park goes to play (chance each look)" = 0.25, 0.0, 1.0, 0.05, 2;
        park_game_every: "Game time in the park about every (s)" = 150.0, 20.0, 3600.0, 10.0, 0;
        park_busy_most: "At most this share of them off on their own things at once" = 0.5, 0.1, 1.0, 0.05, 2;
        park_out: "Out on the desktop at once, at most (mode 2)" = 2.0, 0.0, 12.0, 1.0, 0;
        park_out_every: "Someone goes out about every (s)" = 300.0, 10.0, 7200.0, 10.0, 0;
        park_out_stay: "Stays out (s)" = 180.0, 10.0, 7200.0, 10.0, 0;
    }
    "Roaming", "Weather" {
        carry_keepsake: "Carries its favourite keepsake on its back (share of the time)" = 0.4, 0.0, 1.0, 0.05, 2;
        weather_show: "Weather shown: 0 off, 1 around them, 2 whole screen" = 1.0, 0.0, 2.0, 1.0, 0;
        weather_fps: "Rain and snow over the whole screen, frames a second" = 30.0, 5.0, 144.0, 1.0, 0;
        jolt_drop: "A tumble knocks the umbrella out of the hand" = 0.7, 0.0, 1.0, 0.05, 2;
        parasol_drag: "Open umbrella on the back slows games (speed share)" = 0.85, 0.3, 1.0, 0.05, 2;
        parasol_hang: "Jumps under an open umbrella last (times)" = 1.8, 1.0, 4.0, 0.1, 1;
        parasol_peak: "Under an umbrella, the top of a jump comes at (share of it)" = 0.3, 0.1, 0.5, 0.05, 2;
        parasol_fall: "Gravity under an open umbrella (share)" = 0.22, 0.05, 1.0, 0.05, 2;
        parasol_speed: "Fastest fall under an open umbrella" = 2.0, 0.2, 20.0, 0.1, 1;
        drop_speed: "Shaken or carried this fast (px/s), items fly off" = 1500.0, 200.0, 10000.0, 100.0, 0;
        drop_swing: "Spun this fast (turns), the umbrella flies off" = 10.0, 0.5, 30.0, 0.5, 1;
        snow_settle: "Snow piles up on heads over (s)" = 45.0, 1.0, 600.0, 5.0, 0;
        snow_hop: "Hop after snowflakes (per s)" = 0.06, 0.0, 2.0, 0.01, 2;
        night_sleep: "Go to bed at night (share)" = 0.5, 0.0, 1.0, 0.05, 2;
        night_spread: "Going to bed one by one (s)" = 90.0, 0.0, 1800.0, 5.0, 0;
    }
    "Roaming", "Sleep" {
        sleep_after: "Untouched for (s)" = 300.0, 25.0, 3600.0, 15.0, 0;
        sleep_spread: "Dozing off one by one (s)" = 15.0, 0.0, 120.0, 1.0, 0;
        sleep_sink: "Sinking while lying down" = 0.55, 0.0, 1.0, 0.05, 2;
        nap_games: "Games before a nap" = 3.0, 1.0, 20.0, 1.0, 0;
        nap_energy: "Social battery left for a nap" = 0.25, 0.0, 1.0, 0.05, 2;
        nap_wait: "Nodding off when tired (s)" = 10.0, 0.5, 120.0, 0.5, 1;
        nap_min: "Shortest nap (s)" = 40.0, 5.0, 600.0, 5.0, 0;
        nap_rand: "Random extra nap (s)" = 40.0, 0.0, 600.0, 5.0, 0;
        wake_jump: "Wake with a start" = 0.3, 0.0, 1.0, 0.05, 2;
        wake_min: "Shortest slow wake-up (s)" = 0.3, 0.0, 10.0, 0.1, 1;
        wake_rand: "Random extra wake-up (s)" = 1.5, 0.0, 10.0, 0.1, 1;
    }
    "Roaming", "Reacting to windows" {
        win_curious: "Come to see a new window" = 3.0, 0.0, 12.0, 1.0, 0;
        win_linger: "Looking for (s)" = 5.0, 0.0, 30.0, 0.5, 1;
        win_crowded: "Columns before feeling crowded" = 8.0, 1.0, 40.0, 1.0, 0;
        sit_every: "Sitting down when crowded, per character (s)" = 120.0, 5.0, 1200.0, 5.0, 0;
        sit_min: "Shortest sit (s)" = 8.0, 1.0, 120.0, 1.0, 0;
        sit_rand: "Random extra sit (s)" = 12.0, 0.0, 120.0, 1.0, 0;
        sit_sink: "Lowering when sitting" = 0.2, 0.0, 1.0, 0.05, 2;
    }
    "Social", "Friendship and rivalry" {
        close_friend: "Counts as a close friend" = 1.5, 0.1, 3.0, 0.1, 1;
        close_follow: "Close friend moves along" = 0.7, 0.0, 1.0, 0.05, 2;
        seek_few: "Look for company below (characters here)" = 6.0, 1.0, 20.0, 1.0, 0;
        seek_every: "Looking for someone, with few around (s)" = 40.0, 2.0, 600.0, 2.0, 0;
        meet_friend: "Friendship worth walking over for" = 0.4, 0.0, 3.0, 0.05, 2;
        first_meet: "Friendship from first meeting" = 0.08, 0.0, 1.0, 0.01, 2;
        intro_bond: "Friendship from an introduction" = 0.25, 0.0, 2.0, 0.05, 2;
        intro_call: "Calling a friend over (per minute)" = 1.0, 0.0, 30.0, 0.5, 1;
        bond_minutes: "Chat for one friendship point (min)" = 5.0, 0.5, 120.0, 0.5, 1;
        bond_pull: "Pull towards friends" = 2.0, 0.0, 10.0, 0.5, 1;
        bond_friend: "Friendship for a happy hello" = 0.5, 0.0, 3.0, 0.1, 1;
        bond_game: "Rivalry per game" = 0.25, 0.0, 3.0, 0.05, 2;
        bond_rival: "Rivals want to play" = 1.5, 0.0, 10.0, 0.5, 1;
    }
    "Roaming", "What to do next (weights relative to each other)" {
        walk_chance: "Walk" = 0.30, 0.0, 1.0, 0.01, 2;
        idle_chance: "Stand" = 0.63, 0.0, 1.0, 0.01, 2;
        jump_chance: "Jump" = 0.05, 0.0, 1.0, 0.01, 2;
        chase_chance: "Chase the cursor (less in a crowd)" = 0.01, 0.0, 1.0, 0.01, 2;
    }
    "Roaming", "Standing and walking" {
        idle_min: "Shortest stand (s)" = 1.0, 0.0, 30.0, 0.5, 1;
        idle_rand: "Random extra stand (s)" = 3.0, 0.0, 30.0, 0.5, 1;
        walk_speed: "Walking speed" = 0.12, 0.01, 1.0, 0.01, 2;
        short_share: "Short walks" = 0.4, 0.0, 1.0, 0.05, 2;
        mid_share: "Medium walks (rest are long)" = 0.4, 0.0, 1.0, 0.05, 2;
        short_min: "Short walk, at least" = 0.05, 0.0, 2.0, 0.05, 2;
        short_rand: "Short walk, random" = 0.25, 0.0, 2.0, 0.05, 2;
        mid_min: "Medium walk, at least" = 0.3, 0.0, 4.0, 0.05, 2;
        mid_rand: "Medium walk, random" = 0.9, 0.0, 4.0, 0.05, 2;
    }
    "Roaming", "Edges of the area" {
        edge_margin: "Keep away from edges (at most)" = 0.3, 0.0, 1.0, 0.05, 2;
        edge_trip: "Go to the edge anyway" = 0.1, 0.0, 1.0, 0.05, 2;
    }
    "Roaming", "Jumping and chasing" {
        jump_height: "Jump height" = 2.0, 0.2, 6.0, 0.1, 1;
        jump_time: "Jump length (s)" = 0.9, 0.2, 3.0, 0.05, 2;
        chase_speed: "Chasing speed" = 0.4, 0.05, 2.0, 0.05, 2;
        chase_give_up: "Give up chasing (s)" = 6.0, 1.0, 30.0, 0.5, 1;
        catch_radius: "Cursor catch distance" = 0.015, 0.0, 0.2, 0.005, 3;
    }

    "Touch", "Picking up" {
        pick_delay: "Hold to pick up (s)" = 0.15, 0.0, 1.0, 0.01, 2;
        drag_px: "Drag starts after (px)" = 6.0, 0.0, 50.0, 1.0, 0;
    }
    "Touch", "Throwing and falling" {
        gravity: "Gravity" = 40.0, 5.0, 150.0, 1.0, 0;
        wall_bounce: "Bounce off the sides" = 0.6, 0.0, 1.0, 0.05, 2;
        top_bounce: "Bounce off the top" = 0.5, 0.0, 1.0, 0.05, 2;
        fall_high: "Height for a spinning crash" = 4.0, 0.5, 20.0, 0.5, 1;
        plant_time: "Stuck in the ground (s)" = 0.7, 0.1, 5.0, 0.1, 1;
        plant_hop: "Hop when getting up" = 0.6, 0.0, 2.0, 0.05, 2;
        land_hop: "Bounce on landing" = 0.3, 0.0, 2.0, 0.05, 2;
    }
    "Touch", "Petting" {
        pat_keep: "Each click adds (s)" = 0.3, 0.1, 5.0, 0.1, 1;
        pat_fan: "Spread out while petted (widths)" = 0.5, 0.2, 2.0, 0.05, 2;
        pat_gather: "Call those nearby (screens)" = 0.45, 0.0, 2.0, 0.05, 2;
        pat_gather_chance: "Chance nearby ones come" = 0.35, 0.0, 1.0, 0.05, 2;
        pat_mate_chance: "Chance chat mates come" = 0.5, 0.0, 1.0, 0.05, 2;
        pat_side_max: "Most waiting per side" = 3.0, 1.0, 8.0, 1.0, 0;
        pat_walk_max: "Longest walk to queue" = 0.35, 0.05, 2.0, 0.05, 2;
        pat_crowd_speed: "Queueing speed" = 0.2, 0.05, 2.0, 0.05, 2;
        pat_linger: "Wait after petting (s)" = 2.5, 0.0, 15.0, 0.5, 1;
        pat_hop: "Hops while queueing (per s)" = 0.8, 0.0, 5.0, 0.1, 1;
    }
    "Touch", "Clinging to the cursor" {
        cling_min: "Shortest cling (s)" = 1.0, 0.1, 15.0, 0.1, 1;
        cling_rand: "Random extra cling (s)" = 1.0, 0.0, 15.0, 0.1, 1;
        cling_swing: "Swinging while clinging" = 1.0, 0.0, 5.0, 0.1, 1;
        cling_radius: "Cling area (by height)" = 0.4, 0.05, 1.5, 0.05, 2;
    }
    "Touch", "Dropping into a group" {
        drop_time: "Joining after a drop (s)" = 2.5, 0.0, 15.0, 0.5, 1;
        drop_range: "Joining distance (widths)" = 1.0, 0.0, 5.0, 0.1, 1;
    }

    "Social", "Settling in at startup" {
        warm_alone: "Only wandering (s)" = 180.0, 0.0, 3600.0, 10.0, 0;
        warm_small: "Small chats, no games (until s)" = 300.0, 0.0, 3600.0, 10.0, 0;
        warm_race: "No races (until s)" = 600.0, 0.0, 7200.0, 10.0, 0;
    }
    "Social", "Starting a chat" {
        look_every: "Check interval (s)" = 0.4, 0.1, 5.0, 0.1, 1;
        near_chance: "Chance to talk when near" = 0.03, 0.0, 1.0, 0.01, 2;
        near_range: "Counts as near (widths)" = 1.5, 0.5, 6.0, 0.1, 1;
        overlap_chance: "Chance to talk when overlapping" = 0.45, 0.0, 1.0, 0.01, 2;
        overlap: "Counts as overlapping (widths)" = 0.7, 0.1, 1.5, 0.05, 2;
        step_chance: "Step aside if not talking" = 0.2, 0.0, 1.0, 0.05, 2;
        chat_divisor: "Chats at once (team / this)" = 3.0, 1.0, 12.0, 1.0, 0;
    }
    "Social", "Pause after a chat" {
        cool_base: "Wait (s)" = 40.0, 0.0, 600.0, 5.0, 0;
        cool_rand: "Random extra wait (s)" = 50.0, 0.0, 600.0, 5.0, 0;
        overlap_cool: "Wait ignored when overlapping (s)" = 45.0, 0.0, 600.0, 5.0, 0;
        crowd_rest_base: "Break after a race or crowd (s)" = 90.0, 0.0, 900.0, 5.0, 0;
        crowd_rest_rand: "Random extra break (s)" = 60.0, 0.0, 900.0, 5.0, 0;
        big_group: "Counts as a big group" = 5.0, 2.0, 20.0, 1.0, 0;
    }
    "Social", "Talking" {
        talk_min: "Talk length (s)" = 8.0, 1.0, 120.0, 1.0, 0;
        talk_rand: "Random extra talk (s)" = 6.0, 0.0, 120.0, 1.0, 0;
        short_talk_chance: "Quick hello (pairs)" = 0.3, 0.0, 1.0, 0.05, 2;
        short_talk_min: "Hello length (s)" = 4.0, 1.0, 60.0, 1.0, 0;
        short_talk_rand: "Random extra hello (s)" = 3.0, 0.0, 60.0, 1.0, 0;
        crowd_talk: "Extra per person (s)" = 1.5, 0.0, 30.0, 0.5, 1;
    }
    "Social", "Strolling together" {
        stroll_chance: "Chance to stroll" = 0.3, 0.0, 1.0, 0.05, 2;
        stroll_min: "Stroll distance (px)" = 700.0, 0.0, 4000.0, 50.0, 0;
        stroll_rand: "Random extra distance (px)" = 800.0, 0.0, 4000.0, 50.0, 0;
        max_strolls: "Strolls per chat" = 2.0, 0.0, 10.0, 1.0, 0;
        edge_px: "Distance from strip ends (px)" = 200.0, 0.0, 1000.0, 10.0, 0;
    }
    "Social", "Joining a group" {
        join_2: "Join a group of 2" = 0.01, 0.0, 1.0, 0.01, 2;
        join_3: "Of 3" = 0.03, 0.0, 1.0, 0.01, 2;
        join_4: "Of 4" = 0.06, 0.0, 1.0, 0.01, 2;
        join_5: "Of 5" = 0.10, 0.0, 1.0, 0.01, 2;
        join_6: "Of 6" = 0.14, 0.0, 1.0, 0.01, 2;
        join_7: "Of 7" = 0.18, 0.0, 1.0, 0.01, 2;
        join_8: "Of 8 or more" = 0.25, 0.0, 1.0, 0.01, 2;
        join_range: "Noticed from (widths)" = 2.0, 0.5, 20.0, 0.5, 1;
        join_range_add: "Extra per person" = 0.5, 0.0, 5.0, 0.1, 1;
        join_range_max: "At most" = 5.0, 0.5, 40.0, 0.5, 1;
        walk_join: "Joining while walking, factor" = 0.7, 0.0, 1.0, 0.05, 2;
        max_group: "Most in a group (99 = no limit)" = 99.0, 2.0, 99.0, 1.0, 0;
    }

    "Games", "Social energy" {
        energy_walk: "Walk to refill" = 8.0, 0.5, 60.0, 0.5, 1;
        energy_watch_need: "Needed to come and watch" = 0.6, 0.0, 1.0, 0.05, 2;
        energy_watch_cost: "Cost of watching" = 0.5, 0.0, 1.0, 0.05, 2;
        energy_join_need: "Needed to walk over and join" = 0.3, 0.0, 1.0, 0.05, 2;
        energy_join_cost: "Cost of joining" = 0.2, 0.0, 1.0, 0.05, 2;
        energy_game_need: "Group average for a game" = 0.5, 0.0, 1.0, 0.05, 2;
        energy_game_cost: "Cost of a game" = 0.5, 0.0, 1.0, 0.05, 2;
    }
    "Games", "Starting games" {
        carry_chase: "Chat mates walk after a carried one (speed)" = 0.25, 0.1, 3.0, 0.05, 2;
        carry_follow: "They follow it this far (screens)" = 0.35, 0.0, 2.0, 0.05, 2;
        throw_far: "Thrown this far from a game, out of it (widths)" = 6.0, 1.0, 40.0, 0.5, 1;
        game_after: "Start if nobody joins (s)" = 7.0, 0.5, 120.0, 0.5, 1;
        game_chance: "Chat turns into a game" = 0.35, 0.0, 1.0, 0.05, 2;
        game_min: "Fewest players" = 3.0, 2.0, 12.0, 1.0, 0;
        crowd_games: "Fewest for games in a row" = 4.0, 2.0, 12.0, 1.0, 0;
        encore_keen: "Stays for another game, whatever its energy" = 0.25, 0.0, 1.0, 0.05, 2;
        encore_energy: "Extra chance from energy" = 0.65, 0.0, 1.0, 0.05, 2;
        encore_show: "Holds up the next game for (s)" = 2.5, 0.5, 10.0, 0.5, 1;
        max_games: "Most games per group" = 2.0, 1.0, 7.0, 1.0, 0;
        party_min: "Fewest for domino and tag" = 4.0, 2.0, 12.0, 1.0, 0;
        after_game_min: "Chat after a game (s)" = 6.0, 0.0, 60.0, 1.0, 0;
        after_game_rand: "Random extra chat (s)" = 5.0, 0.0, 60.0, 1.0, 0;
        cheer_time: "Winner celebrates (s)" = 3.0, 0.0, 30.0, 0.5, 1;
    }
    "Games", "Gifts" {
        gift_min: "Fewest characters" = 2.0, 2.0, 2.0, 1.0, 0;
        gift_chance: "Gift in a chat of two" = 0.12, 0.0, 1.0, 0.01, 2;
        gift_prank: "Boxing glove box" = 0.35, 0.0, 1.0, 0.05, 2;
        gift_balloon: "Balloon" = 0.2, 0.0, 1.0, 0.05, 2;
        gift_punch_speed: "Punch throw speed" = 1.4, 0.1, 6.0, 0.1, 1;
        gift_punch_rand: "Random extra speed" = 0.6, 0.0, 4.0, 0.1, 1;
        gift_punch_up: "Punch upward force" = 22.0, 2.0, 60.0, 1.0, 0;
        gift_bounces: "Bounces off the ground" = 3.0, 0.0, 10.0, 1.0, 0;
        gift_bounce: "Bounce strength" = 0.62, 0.1, 0.95, 0.01, 2;
        gift_balloon_height: "Balloon lift" = 9.0, 1.0, 30.0, 0.5, 1;
        gift_balloon_time: "Balloon flight (s)" = 2.2, 0.3, 10.0, 0.1, 1;
    }
    "Games", "Carrot grab" {
        carrot_min: "Fewest players" = 3.0, 2.0, 12.0, 1.0, 0;
        carrot_time: "Game time once grabbed (s)" = 15.0, 3.0, 120.0, 1.0, 0;
        carrot_rush: "Rush to the carrot" = 0.36, 0.05, 2.0, 0.01, 2;
        carrot_run: "Holder speed" = 0.4, 0.05, 2.0, 0.01, 2;
        carrot_sprint: "Holder sprint speed" = 0.62, 0.05, 3.0, 0.01, 2;
        carrot_burst: "Sprint before tiring (s)" = 2.5, 0.2, 20.0, 0.1, 1;
        carrot_tired: "Tired for (s)" = 4.0, 0.0, 30.0, 0.5, 1;
        carrot_slow: "Speed when tired" = 0.16, 0.02, 2.0, 0.01, 2;
        carrot_hop: "Holder hop" = 0.9, 0.0, 3.0, 0.05, 2;
        carrot_hop_chance: "Hop over someone near" = 0.85, 0.0, 1.0, 0.05, 2;
        carrot_chase: "Chasers speed" = 0.34, 0.02, 2.0, 0.01, 2;
        carrot_lag: "Chasers reaction delay (s)" = 0.6, 0.0, 5.0, 0.05, 2;
        carrot_smart_every: "Someone gets clever every (s)" = 3.0, 0.5, 30.0, 0.5, 1;
        carrot_smart_time: "Stays clever for (s)" = 1.5, 0.1, 10.0, 0.1, 1;
        carrot_smart_speed: "Clever speed factor" = 1.35, 1.0, 3.0, 0.05, 2;
        carrot_grace: "Safe after grabbing (s)" = 1.0, 0.0, 5.0, 0.1, 1;
    }
    "Games", "Coming to watch" {
        see_range: "Notice from (widths)" = 6.0, 0.5, 40.0, 0.5, 1;
        see_chance: "Chance to notice" = 0.15, 0.0, 1.0, 0.01, 2;
        watch_join: "Join when the game ends" = 0.35, 0.0, 1.0, 0.05, 2;
        watch_stay_min: "Watches at least (s)" = 25.0, 5.0, 600.0, 5.0, 0;
        watch_stay_rand: "Random extra watching (s)" = 35.0, 0.0, 600.0, 5.0, 0;
        chat_max: "Longest any game lasts (s)" = 360.0, 60.0, 3600.0, 10.0, 0;
    }
    "Games", "Splitting a crowd" {
        split_min: "Group big enough to split" = 6.0, 2.0, 20.0, 1.0, 0;
        mood_min: "Group that splits into chat and game" = 6.0, 4.0, 30.0, 1.0, 0;
        mood_after: "Chat before splitting (s)" = 8.0, 0.0, 120.0, 1.0, 0;
        mood_game: "Someone wants a game" = 0.3, 0.0, 1.0, 0.05, 2;
        game_fit: "Pick games that fit the group" = 0.85, 0.0, 6.0, 0.05, 2;
        game_reward: "Win an accessory in games (chance)" = 0.78, 0.0, 1.0, 0.05, 2;
        split_chance: "Split in two after a game" = 0.5, 0.0, 1.0, 0.05, 2;
    }
    "Games", "Jumping contest" {
        contest_min: "Lowest jump" = 0.7, 0.1, 5.0, 0.1, 1;
        contest_rand: "Random extra height" = 1.6, 0.0, 5.0, 0.1, 1;
    }
    "Games", "Cursor game" {
        cursor_game: "Chance" = 0.04, 0.0, 1.0, 0.01, 2;
        cursor_time_min: "Longest time (s)" = 12.0, 3.0, 180.0, 1.0, 0;
        cursor_time_rand: "Random extra (s)" = 6.0, 0.0, 180.0, 1.0, 0;
        cursor_win: "Win by hanging on for (s)" = 3.0, 0.5, 60.0, 0.5, 1;
        cursor_speed: "Walking speed" = 0.14, 0.02, 1.0, 0.01, 2;
        cursor_reach: "Jump when this near (px)" = 45.0, 5.0, 300.0, 5.0, 0;
    }
    "Games", "Long jump" {
        long_min: "Fewest players" = 5.0, 3.0, 12.0, 1.0, 0;
        long_time: "Longest time (s)" = 50.0, 10.0, 300.0, 5.0, 0;
        long_dist_min: "Shortest flight (px)" = 450.0, 50.0, 4000.0, 50.0, 0;
        long_dist_rand: "Random extra flight (px)" = 900.0, 0.0, 4000.0, 50.0, 0;
        long_air_min: "Time in the air (s)" = 0.9, 0.2, 5.0, 0.1, 1;
        long_air_rand: "Random extra (s)" = 0.5, 0.0, 5.0, 0.1, 1;
        long_gap_min: "Between jumps (s)" = 0.6, 0.1, 10.0, 0.1, 1;
        long_gap_rand: "Random extra (s)" = 0.6, 0.0, 10.0, 0.1, 1;
        long_judge: "Referee looks for (s)" = 1.0, 0.1, 10.0, 0.1, 1;
        long_head_first: "Land head first" = 0.4, 0.0, 1.0, 0.05, 2;
    }
    "Games", "Domino" {
        domino_gap: "Topple interval (s)" = 0.3, 0.05, 3.0, 0.05, 2;
        domino_survive: "Last one stays up" = 0.35, 0.0, 1.0, 0.05, 2;
        domino_rest: "Lying down for (s)" = 1.8, 0.0, 15.0, 0.5, 1;
    }
    "Games", "Volleyball" {
        volley_min: "Fewest players" = 7.0, 6.0, 12.0, 1.0, 0;
        volley_weight: "Pick weight" = 0.65, 0.0, 3.0, 0.05, 2;
        volley_hold: "Ball held this long, someone knocks it out (s)" = 1.5, 0.5, 30.0, 0.5, 1;
        volley_reach: "Hit reach (px)" = 55.0, 10.0, 300.0, 5.0, 0;
        volley_big: "Fewest for a net of 4" = 11.0, 7.0, 30.0, 1.0, 0;
        volley_fumble: "Miss the ball (more in a crowd)" = 0.04, 0.0, 1.0, 0.01, 2;
        volley_time_min: "Shortest hit flight (s)" = 0.7, 0.3, 3.0, 0.05, 2;
        volley_time_rand: "Random extra (s)" = 1.0, 0.0, 3.0, 0.05, 2;
        volley_win: "Points to win" = 3.0, 1.0, 21.0, 1.0, 0;
        volley_speed: "Top player speed" = 0.7, 0.1, 2.0, 0.05, 2;
        volley_net: "Hits that can catch the net" = 0.25, 0.0, 1.0, 0.01, 2;
        volley_gather: "Walking to positions" = 0.18, 0.05, 1.0, 0.01, 2;
        volley_accel: "Acceleration" = 4.0, 0.2, 10.0, 0.1, 1;
    }
    "Games", "Samba" {
        samba_min: "Fewest characters" = 3.0, 2.0, 12.0, 1.0, 0;
        samba_big: "Characters for full chance" = 7.0, 3.0, 30.0, 1.0, 0;
        samba_weight_small: "Pick weight with few" = 0.3, 0.0, 3.0, 0.01, 2;
        samba_weight_big: "Pick weight with many" = 0.75, 0.0, 3.0, 0.01, 2;
        samba_join: "Onlooker joins" = 0.5, 0.0, 1.0, 0.05, 2;
        samba_join_gap: "Between joins (s)" = 4.0, 0.5, 60.0, 0.5, 1;
        samba_time: "Length (s)" = 45.0, 2.0, 120.0, 1.0, 0;
        samba_beat: "Step interval (s)" = 0.45, 0.35, 2.0, 0.05, 2;
        samba_step: "Step size (px)" = 40.0, 0.0, 300.0, 5.0, 0;
    }
    "Games", "Tug of war" {
        tug_min: "Fewest for tug of war" = 2.0, 2.0, 12.0, 1.0, 0;
        tug_pair_chance: "Game chance in a chat of two, factor" = 0.4, 0.0, 1.0, 0.05, 2;
        tug_cheat_chance: "An onlooker cheats" = 0.15, 0.0, 1.0, 0.05, 2;
        tug_cheat_range: "Cheater notices from (screens)" = 0.6, 0.05, 3.0, 0.05, 2;
        tug_cheat_speed: "Cheater running speed" = 0.5, 0.05, 3.0, 0.05, 2;
        tug_win: "Winning distance (px)" = 180.0, 20.0, 1500.0, 10.0, 0;
        tug_time: "Longest time (s)" = 16.0, 3.0, 120.0, 1.0, 0;
        tug_pull: "Pulling strength" = 90.0, 5.0, 500.0, 5.0, 0;
        tug_speed: "Top speed (px/s)" = 90.0, 5.0, 800.0, 5.0, 0;
        tug_cursor: "Pulling with the cursor" = 1.0, 0.0, 5.0, 0.1, 1;
        tug_resist_time: "Resisting the cursor (s)" = 4.0, 0.1, 30.0, 0.5, 1;
        tug_cut_clicks: "Clicks to snap the rope" = 6.0, 2.0, 30.0, 1.0, 0;
        tug_lean: "Leaning back" = 0.22, 0.0, 0.8, 0.02, 2;
        tug_fling_min: "Losers fly (px)" = 400.0, 0.0, 3000.0, 50.0, 0;
        tug_fling_rand: "Random extra (px)" = 350.0, 0.0, 3000.0, 50.0, 0;
    }
    "Games", "Wrestling" {
        wrestle_min: "Fewest for wrestling" = 4.0, 2.0, 12.0, 1.0, 0;
        wrestle_win: "Ring out distance (px)" = 110.0, 20.0, 600.0, 10.0, 0;
        wrestle_round: "Longest bout (s)" = 8.0, 2.0, 30.0, 0.5, 1;
        wrestle_push: "Push strength" = 52.0, 10.0, 400.0, 5.0, 0;
        wrestle_lean: "Leaning in" = 0.24, 0.0, 0.7, 0.02, 2;
        wrestle_fling_min: "Losers fly (px)" = 220.0, 50.0, 1500.0, 25.0, 0;
        wrestle_fling_rand: "Random extra (px)" = 160.0, 0.0, 1500.0, 25.0, 0;
    }
    "Games", "Tag" {
        tag_min: "Fewest players" = 3.0, 2.0, 12.0, 1.0, 0;
        tag_time_min: "Length (s)" = 18.0, 3.0, 180.0, 1.0, 0;
        tag_time_rand: "Random extra (s)" = 7.0, 0.0, 180.0, 1.0, 0;
        tag_pace: "Speeding up and slowing down" = 0.45, 0.0, 0.9, 0.05, 2;
        tag_wait: "It waits before starting (s)" = 2.0, 0.0, 15.0, 0.5, 1;
        tag_delay: "It turns back after (s)" = 0.45, 0.0, 3.0, 0.05, 2;
        tag_leap_range: "Leap out of a corner from (px)" = 350.0, 0.0, 2000.0, 25.0, 0;
        tag_leap_pace: "Speed needed to leap (vs normal)" = 1.05, 0.5, 2.0, 0.05, 2;
        tag_run: "Running away speed" = 0.26, 0.02, 2.0, 0.01, 2;
        tag_it: "Speed of whoever is it" = 0.34, 0.02, 2.0, 0.01, 2;
    }

    "Race", "Starting by itself" {
        race_call: "Talk after the call (s)" = 3.0, 0.5, 30.0, 0.5, 1;
        race_invite_pause: "Pause after inviting (s)" = 2.0, 0.0, 10.0, 0.1, 1;
        race_herald_speed: "Inviter walking speed" = 0.18, 0.05, 2.0, 0.05, 2;
        race_wave_min: "Group joins the race every (s)" = 0.2, 0.05, 5.0, 0.05, 2;
        race_wave_rand: "Random extra (s)" = 0.3, 0.0, 5.0, 0.05, 2;
        race_cool: "Between two races, at least (s)" = 1800.0, 0.0, 86400.0, 60.0, 0;
        race_group_min: "Group that can decide: fewest" = 3.0, 2.0, 12.0, 1.0, 0;
        race_group_max: "Group that can decide: most" = 4.0, 2.0, 30.0, 1.0, 0;
        race_ask_time: "Waiting for an answer (s)" = 1.0, 0.1, 10.0, 0.1, 1;
        race_refuse_min: "Fewest characters before refusing" = 9.0, 2.0, 50.0, 1.0, 0;
        race_refuse: "Refuse the invitation" = 0.03, 0.0, 1.0, 0.01, 2;
        race_watch_speed: "Onlookers walking speed" = 0.3, 0.05, 2.0, 0.05, 2;
        race_watch_cheer: "Onlookers hop (per s)" = 2.0, 0.0, 10.0, 0.5, 1;
        race_chance: "Small group decides to race (per chat)" = 0.02, 0.0, 1.0, 0.05, 2;
    }
    "Race", "Gathering" {
        race_gather_speed: "Walking to gather" = 0.22, 0.02, 2.0, 0.01, 2;
        race_gather_rand: "Random extra speed" = 0.08, 0.0, 2.0, 0.01, 2;
        race_mill_speed: "Milling about speed" = 0.06, 0.01, 1.0, 0.01, 2;
        race_mill_rand: "Random extra speed" = 0.06, 0.0, 1.0, 0.01, 2;
        race_zone: "Gathering area (share of track, at most 600 px)" = 0.25, 0.05, 1.0, 0.05, 2;
        race_wait: "Waiting for latecomers (s)" = 90.0, 5.0, 600.0, 5.0, 0;
        race_countdown: "Before the start (s)" = 0.6, 0.0, 10.0, 0.1, 1;
    }
    "Race", "Running" {
        race_pace: "Slowest speed" = 0.2, 0.02, 2.0, 0.01, 2;
        race_pace_range: "Speed range" = 0.14, 0.0, 2.0, 0.01, 2;
        race_pace_gap: "Speed changes every (s)" = 0.8, 0.1, 10.0, 0.1, 1;
        race_hop_chance: "Hop over someone" = 0.5, 0.0, 1.0, 0.05, 2;
        race_leaders: "Leaders who may hop" = 3.0, 0.0, 12.0, 1.0, 0;
        race_hit_chance: "Knock the one ahead flying" = 0.15, 0.0, 1.0, 0.05, 2;
        race_hit_min: "Knocked flying (px)" = 200.0, 0.0, 3000.0, 25.0, 0;
        race_hit_rand: "Random extra (px)" = 300.0, 0.0, 3000.0, 25.0, 0;
        race_cheer: "Winner celebrates (s)" = 5.0, 0.0, 30.0, 0.5, 1;
    }
}

impl Default for Tuning {
    fn default() -> Self {
        Self::DEFAULT
    }
}

static DEFAULTS: Tuning = Tuning::DEFAULT;
static CURRENT: AtomicPtr<Tuning> = AtomicPtr::new(std::ptr::addr_of!(DEFAULTS) as *mut Tuning);

/// The numbers in force right now. Read many times a frame, so no lock and
/// no copy.
#[inline]
pub fn get() -> &'static Tuning {
    // SAFETY: only ever points at DEFAULTS or a leaked Box, never freed
    unsafe { &*CURRENT.load(Ordering::Acquire) }
}

/// Only runs when someone changes a setting, so the old values (a few
/// hundred bytes) are simply left behind: a reader may still be using them.
pub fn set(t: Tuning) {
    if *get() != t {
        CURRENT.store(Box::into_raw(Box::new(t)), Ordering::Release);
    }
}

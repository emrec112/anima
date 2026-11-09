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
    "Gezinme", "Pencereye tırmanma (Windows)" {
        climb_every: "Kişi başı deneme aralığı (sn)" = 60.0, 5.0, 600.0, 5.0, 0;
        climb_share: "Aynı anda çıkabilen oran" = 0.3, 0.05, 1.0, 0.05, 2;
        climb_speed: "Tırmanma hızı" = 1.6, 0.2, 10.0, 0.1, 1;
        climb_stay_min: "Üstte en az kalma (sn)" = 15.0, 1.0, 300.0, 1.0, 0;
        climb_stay_rand: "Kalmaya eklenen rastgele (sn)" = 30.0, 0.0, 300.0, 1.0, 0;
        climb_min_width: "En dar pencere (px)" = 180.0, 50.0, 1000.0, 10.0, 0;
        climb_slip: "Tırmanırken düşme ihtimali" = 0.25, 0.0, 1.0, 0.05, 2;
        climb_hop: "Başka pencereye atlama ihtimali" = 0.6, 0.0, 1.0, 0.05, 2;
        climb_leap: "Atlama yüksekliği" = 1.2, 0.0, 6.0, 0.1, 1;
        climb_reach: "Atlanabilen boşluk (karakter genişliği)" = 3.0, 0.0, 20.0, 0.5, 1;
        climb_jump_up: "Zıplanabilen yükseklik (karakter boyu)" = 1.3, 0.0, 6.0, 0.1, 1;
        climb_throw: "Fırlatma yüksekliği" = 3.0, 0.0, 12.0, 0.5, 1;
        climb_give_up: "Pencereye yürümeyi bırakma (sn)" = 25.0, 5.0, 120.0, 5.0, 0;
    }
    "Gezinme", "Ne yapacağına karar verme (oranlar birbirine göre)" {
        walk_chance: "Yürüme" = 0.30, 0.0, 1.0, 0.01, 2;
        idle_chance: "Durma" = 0.63, 0.0, 1.0, 0.01, 2;
        jump_chance: "Zıplama" = 0.05, 0.0, 1.0, 0.01, 2;
        chase_chance: "İmleci kovalama (kalabalıkta azalır)" = 0.01, 0.0, 1.0, 0.01, 2;
    }
    "Gezinme", "Durma ve yürüme" {
        idle_min: "En kısa durma (sn)" = 1.0, 0.0, 30.0, 0.5, 1;
        idle_rand: "Durmaya eklenen rastgele (sn)" = 3.0, 0.0, 30.0, 0.5, 1;
        walk_speed: "Yürüme hızı" = 0.12, 0.01, 1.0, 0.01, 2;
        short_share: "Kısa yürüyüş payı" = 0.4, 0.0, 1.0, 0.05, 2;
        mid_share: "Orta yürüyüş payı (kalanı uzun)" = 0.4, 0.0, 1.0, 0.05, 2;
        short_min: "Kısa yürüyüş en az" = 0.05, 0.0, 2.0, 0.05, 2;
        short_rand: "Kısa yürüyüş rastgele" = 0.25, 0.0, 2.0, 0.05, 2;
        mid_min: "Orta yürüyüş en az" = 0.3, 0.0, 4.0, 0.05, 2;
        mid_rand: "Orta yürüyüş rastgele" = 0.9, 0.0, 4.0, 0.05, 2;
    }
    "Gezinme", "Alanın kenarları" {
        edge_margin: "Kenardan uzak durma (en çok)" = 0.3, 0.0, 1.0, 0.05, 2;
        edge_trip: "Yine de kenara gitme oranı" = 0.1, 0.0, 1.0, 0.05, 2;
    }
    "Gezinme", "Zıplama ve kovalama" {
        jump_height: "Zıplama yüksekliği" = 2.0, 0.2, 6.0, 0.1, 1;
        jump_time: "Zıplama süresi (sn)" = 0.9, 0.2, 3.0, 0.05, 2;
        chase_speed: "Kovalama hızı" = 0.4, 0.05, 2.0, 0.05, 2;
        chase_give_up: "Kovalamadan vazgeçme (sn)" = 6.0, 1.0, 30.0, 0.5, 1;
        catch_radius: "İmleci yakalama yakınlığı" = 0.015, 0.0, 0.2, 0.005, 3;
    }

    "Etkileşim", "Tutup kaldırma" {
        pick_delay: "Basılı tutma süresi (sn)" = 0.15, 0.0, 1.0, 0.01, 2;
        drag_px: "Sürüklemeye başlama (piksel)" = 6.0, 0.0, 50.0, 1.0, 0;
    }
    "Etkileşim", "Fırlatma ve düşme" {
        gravity: "Yerçekimi" = 40.0, 5.0, 150.0, 1.0, 0;
        wall_bounce: "Kenardan sekme" = 0.6, 0.0, 1.0, 0.05, 2;
        top_bounce: "Tavandan sekme" = 0.5, 0.0, 1.0, 0.05, 2;
        fall_high: "Dönerek düşüp saplanma yüksekliği" = 4.0, 0.5, 20.0, 0.5, 1;
        plant_time: "Saplı kalma süresi (sn)" = 0.7, 0.1, 5.0, 0.1, 1;
        plant_hop: "Kalkarken zıplama" = 0.6, 0.0, 2.0, 0.05, 2;
        land_hop: "İnişte seken zıplama" = 0.3, 0.0, 2.0, 0.05, 2;
    }
    "Etkileşim", "Sevme" {
        pat_keep: "Her tıklamada uzama (sn)" = 0.6, 0.1, 5.0, 0.1, 1;
    }
    "Etkileşim", "İmlece tutunma" {
        cling_min: "En kısa tutunma (sn)" = 1.0, 0.1, 15.0, 0.1, 1;
        cling_rand: "Tutunmaya eklenen rastgele (sn)" = 1.0, 0.0, 15.0, 0.1, 1;
        cling_swing: "Tutunurken savrulma" = 1.0, 0.0, 5.0, 0.1, 1;
        cling_radius: "Tutunma alanı (karakter boyuna göre)" = 0.4, 0.05, 1.5, 0.05, 2;
    }
    "Etkileşim", "Elle gruba bırakma" {
        drop_time: "Bırakınca katılma süresi (sn)" = 2.5, 0.0, 15.0, 0.5, 1;
        drop_range: "Katılma mesafesi (karakter genişliği)" = 1.0, 0.0, 5.0, 0.1, 1;
    }

    "Sohbet", "Açılışta alışma" {
        warm_alone: "Sadece gezinme (sn)" = 180.0, 0.0, 3600.0, 10.0, 0;
        warm_small: "Küçük sohbetler, oyun yok (sn'ye kadar)" = 300.0, 0.0, 3600.0, 10.0, 0;
        warm_race: "Yarış olmadan (sn'ye kadar)" = 600.0, 0.0, 7200.0, 10.0, 0;
    }
    "Sohbet", "Sohbetin başlaması" {
        look_every: "Yoklama aralığı (sn)" = 0.4, 0.1, 5.0, 0.1, 1;
        near_chance: "Yakın duranların konuşma şansı" = 0.03, 0.0, 1.0, 0.01, 2;
        near_range: "Yakın sayılma (karakter genişliği)" = 1.5, 0.5, 6.0, 0.1, 1;
        overlap_chance: "İç içe duranların konuşma şansı" = 0.45, 0.0, 1.0, 0.01, 2;
        overlap: "İç içe sayılma (karakter genişliği)" = 0.7, 0.1, 1.5, 0.05, 2;
        step_chance: "Konuşmazsa kenara çekilme şansı" = 0.2, 0.0, 1.0, 0.05, 2;
        chat_divisor: "Aynı anda sohbet (ekip / bu sayı)" = 3.0, 1.0, 12.0, 1.0, 0;
    }
    "Sohbet", "Sohbetten sonra bekleme" {
        cool_base: "Bekleme (sn)" = 40.0, 0.0, 600.0, 5.0, 0;
        cool_rand: "Beklemeye eklenen rastgele (sn)" = 50.0, 0.0, 600.0, 5.0, 0;
        overlap_cool: "İç içeyken yok sayılan bekleme (sn)" = 45.0, 0.0, 600.0, 5.0, 0;
        crowd_rest_base: "Yarış / büyük grup sonrası mola (sn)" = 90.0, 0.0, 900.0, 5.0, 0;
        crowd_rest_rand: "Molaya eklenen rastgele (sn)" = 60.0, 0.0, 900.0, 5.0, 0;
        big_group: "Büyük grup sayılma (kişi)" = 5.0, 2.0, 20.0, 1.0, 0;
    }
    "Sohbet", "Konuşma" {
        talk_min: "Konuşma süresi (sn)" = 8.0, 1.0, 120.0, 1.0, 0;
        talk_rand: "Konuşmaya eklenen rastgele (sn)" = 6.0, 0.0, 120.0, 1.0, 0;
        short_talk_chance: "Kısa selamlaşma şansı (ikililer)" = 0.3, 0.0, 1.0, 0.05, 2;
        short_talk_min: "Selamlaşma süresi (sn)" = 4.0, 1.0, 60.0, 1.0, 0;
        short_talk_rand: "Selamlaşmaya eklenen rastgele (sn)" = 3.0, 0.0, 60.0, 1.0, 0;
        crowd_talk: "Kişi başına ek süre (sn)" = 1.5, 0.0, 30.0, 0.5, 1;
    }
    "Sohbet", "Birlikte gezme" {
        stroll_chance: "Gezmeye çıkma şansı" = 0.3, 0.0, 1.0, 0.05, 2;
        stroll_min: "Gezme mesafesi (piksel)" = 700.0, 0.0, 4000.0, 50.0, 0;
        stroll_rand: "Mesafeye eklenen rastgele (piksel)" = 800.0, 0.0, 4000.0, 50.0, 0;
        max_strolls: "Sohbet başına gezme" = 2.0, 0.0, 10.0, 1.0, 0;
        edge_px: "Şerit kenarına yaklaşma sınırı (piksel)" = 200.0, 0.0, 1000.0, 10.0, 0;
    }
    "Sohbet", "Gruba katılma" {
        join_2: "2 kişilik gruba katılma şansı" = 0.01, 0.0, 1.0, 0.01, 2;
        join_3: "3 kişilik" = 0.03, 0.0, 1.0, 0.01, 2;
        join_4: "4 kişilik" = 0.06, 0.0, 1.0, 0.01, 2;
        join_5: "5 kişilik" = 0.10, 0.0, 1.0, 0.01, 2;
        join_6: "6 kişilik" = 0.14, 0.0, 1.0, 0.01, 2;
        join_7: "7 kişilik" = 0.18, 0.0, 1.0, 0.01, 2;
        join_8: "8 ve üstü" = 0.25, 0.0, 1.0, 0.01, 2;
        join_range: "Fark edilme mesafesi (karakter genişliği)" = 2.0, 0.5, 20.0, 0.5, 1;
        join_range_add: "Kişi başına ek mesafe" = 0.5, 0.0, 5.0, 0.1, 1;
        join_range_max: "En çok mesafe" = 5.0, 0.5, 40.0, 0.5, 1;
        walk_join: "Yürürken katılma çarpanı" = 0.7, 0.0, 1.0, 0.05, 2;
        max_group: "Bir grupta en çok kişi (99 = sınırsız)" = 99.0, 2.0, 99.0, 1.0, 0;
    }

    "Oyunlar", "Sosyal enerji" {
        energy_walk: "Dolması için yürüme mesafesi" = 8.0, 0.5, 60.0, 0.5, 1;
        energy_watch_need: "İzlemeye gelmek için gereken" = 0.6, 0.0, 1.0, 0.05, 2;
        energy_watch_cost: "İzlemenin bedeli" = 0.5, 0.0, 1.0, 0.05, 2;
        energy_join_need: "Yürüyerek katılmak için gereken" = 0.3, 0.0, 1.0, 0.05, 2;
        energy_join_cost: "Katılmanın bedeli" = 0.2, 0.0, 1.0, 0.05, 2;
        energy_game_need: "Oyun için grup ortalaması" = 0.5, 0.0, 1.0, 0.05, 2;
        energy_game_cost: "Oyunun bedeli" = 0.5, 0.0, 1.0, 0.05, 2;
    }
    "Oyunlar", "Oyunların başlaması" {
        game_after: "Kimse katılmazsa başlama (sn)" = 7.0, 0.5, 120.0, 0.5, 1;
        game_chance: "Sohbetin oyuna dönme şansı" = 0.35, 0.0, 1.0, 0.05, 2;
        game_min: "En az oyuncu" = 3.0, 2.0, 12.0, 1.0, 0;
        crowd_games: "Üst üste oyun için en az kişi" = 4.0, 2.0, 12.0, 1.0, 0;
        max_games: "Grup başına en çok oyun" = 2.0, 1.0, 7.0, 1.0, 0;
        party_min: "Domino, halat, ebelemece için en az" = 4.0, 2.0, 12.0, 1.0, 0;
        after_game_min: "Oyundan sonra sohbet (sn)" = 6.0, 0.0, 60.0, 1.0, 0;
        after_game_rand: "Sohbete eklenen rastgele (sn)" = 5.0, 0.0, 60.0, 1.0, 0;
        cheer_time: "Kazananın sevinmesi (sn)" = 3.0, 0.0, 30.0, 0.5, 1;
    }
    "Oyunlar", "Hediye" {
        gift_min: "En az karakter" = 2.0, 2.0, 2.0, 1.0, 0;
        gift_chance: "İki kişilik sohbette hediye şansı" = 0.12, 0.0, 1.0, 0.01, 2;
        gift_prank: "Yumruklu kutu şansı" = 0.35, 0.0, 1.0, 0.05, 2;
        gift_balloon: "Balon şansı" = 0.2, 0.0, 1.0, 0.05, 2;
        gift_punch_speed: "Yumruğun fırlatma hızı" = 1.4, 0.1, 6.0, 0.1, 1;
        gift_punch_rand: "Eklenen rastgele hız" = 0.6, 0.0, 4.0, 0.1, 1;
        gift_punch_up: "Yumruğun yukarı gücü" = 22.0, 2.0, 60.0, 1.0, 0;
        gift_bounces: "Yerden sekme sayısı" = 3.0, 0.0, 10.0, 1.0, 0;
        gift_bounce: "Sekme gücü" = 0.62, 0.1, 0.95, 0.01, 2;
        gift_balloon_height: "Balonla yükselme" = 9.0, 1.0, 30.0, 0.5, 1;
        gift_balloon_time: "Balonla uçma süresi (sn)" = 2.2, 0.3, 10.0, 0.1, 1;
    }
    "Oyunlar", "Havuç kapmaca" {
        carrot_min: "En az oyuncu" = 3.0, 2.0, 12.0, 1.0, 0;
        carrot_time: "Havucu kapınca oyun süresi (sn)" = 15.0, 3.0, 120.0, 1.0, 0;
        carrot_rush: "Havuca koşma hızı" = 0.36, 0.05, 2.0, 0.01, 2;
        carrot_run: "Havuçlunun normal hızı" = 0.4, 0.05, 2.0, 0.01, 2;
        carrot_sprint: "Havuçlunun depar hızı" = 0.62, 0.05, 3.0, 0.01, 2;
        carrot_burst: "Yorulmadan depar süresi (sn)" = 2.5, 0.2, 20.0, 0.1, 1;
        carrot_tired: "Yorgun kalma süresi (sn)" = 4.0, 0.0, 30.0, 0.5, 1;
        carrot_slow: "Yorgunken hız" = 0.16, 0.02, 2.0, 0.01, 2;
        carrot_hop: "Havuçlunun zıplaması" = 0.9, 0.0, 3.0, 0.05, 2;
        carrot_hop_chance: "Yakındakinin üstünden zıplama şansı" = 0.85, 0.0, 1.0, 0.05, 2;
        carrot_chase: "Kovalayanların hızı" = 0.34, 0.02, 2.0, 0.01, 2;
        carrot_lag: "Kovalayanların tepki gecikmesi (sn)" = 0.6, 0.0, 5.0, 0.05, 2;
        carrot_smart_every: "Biri zekileşme aralığı (sn)" = 3.0, 0.5, 30.0, 0.5, 1;
        carrot_smart_time: "Zeki kalma süresi (sn)" = 1.5, 0.1, 10.0, 0.1, 1;
        carrot_smart_speed: "Zekiyken hız çarpanı" = 1.35, 1.0, 3.0, 0.05, 2;
        carrot_grace: "Kapınca dokunulmazlık (sn)" = 1.0, 0.0, 5.0, 0.1, 1;
    }
    "Oyunlar", "Oyunu izlemeye gelme" {
        see_range: "Fark etme mesafesi (karakter genişliği)" = 6.0, 0.5, 40.0, 0.5, 1;
        see_chance: "Fark etme şansı" = 0.15, 0.0, 1.0, 0.01, 2;
        watch_join: "Oyun bitince katılma şansı" = 0.35, 0.0, 1.0, 0.05, 2;
    }
    "Oyunlar", "Kalabalığın bölünmesi" {
        split_min: "Bölünebilen grup (kişi)" = 6.0, 2.0, 20.0, 1.0, 0;
        mood_min: "Sohbet ve oyun diye ayrılan grup (kişi)" = 6.0, 4.0, 30.0, 1.0, 0;
        mood_after: "Ayrılmadan önce sohbet (sn)" = 8.0, 0.0, 120.0, 1.0, 0;
        mood_game: "Birinin oyun isteme şansı" = 0.3, 0.0, 1.0, 0.05, 2;
        game_fit: "Kişi sayısına uyan oyunu seçme gücü" = 2.0, 0.0, 6.0, 0.5, 1;
        split_chance: "Oyun sonrası ikiye bölünme şansı" = 0.5, 0.0, 1.0, 0.05, 2;
    }
    "Oyunlar", "Zıplama oyunu" {
        contest_min: "En alçak zıplama" = 0.7, 0.1, 5.0, 0.1, 1;
        contest_rand: "Eklenen rastgele yükseklik" = 1.6, 0.0, 5.0, 0.1, 1;
    }
    "Oyunlar", "İmleç oyunu" {
        cursor_game: "Çıkma şansı" = 0.07, 0.0, 1.0, 0.01, 2;
        cursor_time_min: "En uzun süre (sn)" = 12.0, 3.0, 180.0, 1.0, 0;
        cursor_time_rand: "Eklenen rastgele (sn)" = 6.0, 0.0, 180.0, 1.0, 0;
        cursor_win: "Bu kadar tutunan kazanır (sn)" = 3.0, 0.5, 60.0, 0.5, 1;
        cursor_speed: "Yürüme hızı" = 0.14, 0.02, 1.0, 0.01, 2;
        cursor_reach: "Zıplama yakınlığı (piksel)" = 45.0, 5.0, 300.0, 5.0, 0;
    }
    "Oyunlar", "İleri atlama" {
        long_min: "En az oyuncu" = 5.0, 3.0, 12.0, 1.0, 0;
        long_time: "En uzun süre (sn)" = 50.0, 10.0, 300.0, 5.0, 0;
        long_dist_min: "En kısa uçuş (piksel)" = 450.0, 50.0, 4000.0, 50.0, 0;
        long_dist_rand: "Uçuşa eklenen rastgele (piksel)" = 900.0, 0.0, 4000.0, 50.0, 0;
        long_air_min: "Havada kalma (sn)" = 0.9, 0.2, 5.0, 0.1, 1;
        long_air_rand: "Eklenen rastgele (sn)" = 0.5, 0.0, 5.0, 0.1, 1;
        long_gap_min: "Atlayışlar arası (sn)" = 0.6, 0.1, 10.0, 0.1, 1;
        long_gap_rand: "Eklenen rastgele (sn)" = 0.6, 0.0, 10.0, 0.1, 1;
        long_judge: "Hakemin bakma süresi (sn)" = 1.0, 0.1, 10.0, 0.1, 1;
        long_head_first: "Tepetaklak saplanma şansı" = 0.4, 0.0, 1.0, 0.05, 2;
    }
    "Oyunlar", "Domino" {
        domino_gap: "Devrilme aralığı (sn)" = 0.3, 0.05, 3.0, 0.05, 2;
        domino_survive: "Sondakinin kurtulma şansı" = 0.35, 0.0, 1.0, 0.05, 2;
        domino_rest: "Yerde kalma (sn)" = 1.8, 0.0, 15.0, 0.5, 1;
    }
    "Oyunlar", "Voleybol" {
        volley_min: "En az karakter (çift sayı)" = 6.0, 6.0, 12.0, 2.0, 0;
        volley_reach: "Vuruş alanı (piksel)" = 55.0, 10.0, 300.0, 5.0, 0;
        volley_big: "4 kişilik ağ için en az karakter" = 11.0, 7.0, 30.0, 1.0, 0;
        volley_fumble: "Topu kaçırma şansı (kalabalıkta artar)" = 0.04, 0.0, 1.0, 0.01, 2;
        volley_time_min: "Vuruşun en kısa havada kalışı (sn)" = 0.7, 0.3, 3.0, 0.05, 2;
        volley_time_rand: "Eklenen rastgele (sn)" = 1.0, 0.0, 3.0, 0.05, 2;
        volley_win: "Kazanmak için sayı" = 5.0, 1.0, 21.0, 1.0, 0;
        volley_speed: "Oyuncuların en yüksek hızı" = 0.7, 0.1, 2.0, 0.05, 2;
        volley_net: "Ağa takılabilen vuruş oranı" = 0.25, 0.0, 1.0, 0.01, 2;
        volley_gather: "Toplanırken yürüme hızı" = 0.18, 0.05, 1.0, 0.01, 2;
        volley_accel: "Hızlanma" = 4.0, 0.2, 10.0, 0.1, 1;
    }
    "Oyunlar", "Samba" {
        samba_min: "En az karakter" = 3.0, 2.0, 12.0, 1.0, 0;
        samba_big: "Tam şans için kişi sayısı" = 7.0, 3.0, 30.0, 1.0, 0;
        samba_weight_small: "Az kişiyle seçilme ağırlığı" = 0.12, 0.0, 3.0, 0.01, 2;
        samba_weight_big: "Çok kişiyle seçilme ağırlığı" = 0.3, 0.0, 3.0, 0.01, 2;
        samba_join: "Görenin katılma şansı" = 0.5, 0.0, 1.0, 0.05, 2;
        samba_join_gap: "Katılımlar arası (sn)" = 4.0, 0.5, 60.0, 0.5, 1;
        samba_time: "Süre (sn)" = 45.0, 2.0, 120.0, 1.0, 0;
        samba_beat: "Adım aralığı (sn)" = 0.45, 0.35, 2.0, 0.05, 2;
        samba_step: "Adım boyu (piksel)" = 40.0, 0.0, 300.0, 5.0, 0;
    }
    "Oyunlar", "Halat çekme" {
        tug_win: "Kazanma mesafesi (piksel)" = 180.0, 20.0, 1500.0, 10.0, 0;
        tug_time: "En uzun süre (sn)" = 16.0, 3.0, 120.0, 1.0, 0;
        tug_pull: "Çekiş gücü" = 90.0, 5.0, 500.0, 5.0, 0;
        tug_speed: "En yüksek hız (piksel/sn)" = 90.0, 5.0, 800.0, 5.0, 0;
        tug_cursor: "İmleçle çekme gücü" = 1.0, 0.0, 5.0, 0.1, 1;
        tug_resist_time: "İmlece direnme süresi (sn)" = 4.0, 0.1, 30.0, 0.5, 1;
        tug_cut_clicks: "Halatın kopması için tıklama" = 6.0, 2.0, 30.0, 1.0, 0;
        tug_lean: "Geriye yaslanma" = 0.22, 0.0, 0.8, 0.02, 2;
        tug_fling_min: "Kaybedenin savrulması (piksel)" = 400.0, 0.0, 3000.0, 50.0, 0;
        tug_fling_rand: "Savrulmaya eklenen rastgele (piksel)" = 350.0, 0.0, 3000.0, 50.0, 0;
    }
    "Oyunlar", "Ebelemece" {
        tag_time_min: "Süre (sn)" = 18.0, 3.0, 180.0, 1.0, 0;
        tag_time_rand: "Eklenen rastgele (sn)" = 7.0, 0.0, 180.0, 1.0, 0;
        tag_pace: "Hızlanıp yavaşlama" = 0.45, 0.0, 0.9, 0.05, 2;
        tag_wait: "Ebenin başlamadan beklemesi (sn)" = 2.0, 0.0, 15.0, 0.5, 1;
        tag_delay: "Ebenin geri dönme gecikmesi (sn)" = 0.45, 0.0, 3.0, 0.05, 2;
        tag_leap_range: "Köşeden zıplayarak kaçma mesafesi (piksel)" = 350.0, 0.0, 2000.0, 25.0, 0;
        tag_leap_pace: "Zıplamak için gereken hız (normale göre)" = 1.05, 0.5, 2.0, 0.05, 2;
        tag_run: "Kaçma hızı" = 0.26, 0.02, 2.0, 0.01, 2;
        tag_it: "Ebenin hızı" = 0.34, 0.02, 2.0, 0.01, 2;
    }

    "Yarış", "Kendiliğinden başlama" {
        race_call: "Duyurudan sonra konuşma (sn)" = 3.0, 0.5, 30.0, 0.5, 1;
        race_invite_pause: "Davetten sonra bekleme (sn)" = 2.0, 0.0, 10.0, 0.1, 1;
        race_herald_speed: "Davet edenin yürüme hızı" = 0.18, 0.05, 2.0, 0.05, 2;
        race_wave_min: "Grubun yarışa geçme aralığı (sn)" = 0.2, 0.05, 5.0, 0.05, 2;
        race_wave_rand: "Eklenen rastgele (sn)" = 0.3, 0.0, 5.0, 0.05, 2;
        race_cool: "İki yarış arası en az (sn)" = 1800.0, 0.0, 86400.0, 60.0, 0;
        race_group_min: "Karar verebilen grup: en az" = 3.0, 2.0, 12.0, 1.0, 0;
        race_group_max: "Karar verebilen grup: en çok" = 4.0, 2.0, 30.0, 1.0, 0;
        race_ask_time: "Cevabı bekleme (sn)" = 1.0, 0.1, 10.0, 0.1, 1;
        race_refuse_min: "Reddetme için en az karakter" = 9.0, 2.0, 50.0, 1.0, 0;
        race_refuse: "Daveti reddetme şansı" = 0.03, 0.0, 1.0, 0.01, 2;
        race_watch_speed: "İzleyicilerin yürüme hızı" = 0.3, 0.05, 2.0, 0.05, 2;
        race_watch_cheer: "İzleyicilerin zıplaması (saniyede)" = 2.0, 0.0, 10.0, 0.5, 1;
        race_chance: "Küçük grubun yarışa karar verme şansı (sohbet başına)" = 0.02, 0.0, 1.0, 0.05, 2;
    }
    "Yarış", "Toplanma" {
        race_gather_speed: "Toplanmaya yürüme hızı" = 0.22, 0.02, 2.0, 0.01, 2;
        race_gather_rand: "Eklenen rastgele hız" = 0.08, 0.0, 2.0, 0.01, 2;
        race_mill_speed: "Alanda dolaşma hızı" = 0.06, 0.01, 1.0, 0.01, 2;
        race_mill_rand: "Eklenen rastgele hız" = 0.06, 0.0, 1.0, 0.01, 2;
        race_zone: "Toplanma alanı (pistin oranı, en çok 600 piksel)" = 0.25, 0.05, 1.0, 0.05, 2;
        race_wait: "Gelmeyenleri bekleme (sn)" = 90.0, 5.0, 600.0, 5.0, 0;
        race_countdown: "Başlamadan önce (sn)" = 0.6, 0.0, 10.0, 0.1, 1;
    }
    "Yarış", "Koşu" {
        race_pace: "En yavaş hız" = 0.2, 0.02, 2.0, 0.01, 2;
        race_pace_range: "Hız aralığı" = 0.14, 0.0, 2.0, 0.01, 2;
        race_pace_gap: "Hız değişme aralığı (sn)" = 0.8, 0.1, 10.0, 0.1, 1;
        race_hop_chance: "Üstünden zıplama şansı" = 0.5, 0.0, 1.0, 0.05, 2;
        race_leaders: "Zıplayabilen öndekiler" = 3.0, 0.0, 12.0, 1.0, 0;
        race_hit_chance: "Öndekine vurup fırlatma şansı" = 0.15, 0.0, 1.0, 0.05, 2;
        race_hit_min: "Vurulanın savrulması (piksel)" = 200.0, 0.0, 3000.0, 25.0, 0;
        race_hit_rand: "Savrulmaya eklenen rastgele (piksel)" = 300.0, 0.0, 3000.0, 25.0, 0;
        race_cheer: "Kazananın sevinmesi (sn)" = 5.0, 0.0, 30.0, 0.5, 1;
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

//! The Characters tab: the line-up on the left, characters to add on the
//! right. Changes wait for Apply; right-click a character to see what it's
//! doing and who its friends are.

use crate::pages::{Ui, load_texture, picture};
use crate::{config, daemon, team};
use adw::prelude::*;
use gtk::{gdk, glib};
use std::path::{Path, PathBuf};
use std::rc::Rc;

struct Roster {
    ui: Rc<Ui>,
    cards: gtk::FlowBox,
    card_rows: std::cell::RefCell<Vec<(String, gtk::Box, gtk::Switch)>>,
    summary: gtk::Label,
    apply: gtk::Button,
    pending: gtk::Label,
    lineups: gtk::Box,
}

fn name_of(path: &str) -> String {
    Path::new(path).parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn small(text: &str) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).build();
    l.add_css_class("dim-label");
    l.add_css_class("caption");
    l
}

fn clear_flow(f: &gtk::FlowBox) {
    while let Some(c) = f.first_child() {
        f.remove(&c);
    }
}

fn clear_box(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn thumb(ui: &Ui, path: &str, size: i32) -> gtk::Widget {
    let key = format!("{size}:{path}");
    let mut cache = ui.team_thumbs.borrow_mut();
    if !cache.contains_key(&key) {
        if let Some(t) = load_texture(Path::new(path), size) {
            cache.insert(key.clone(), t);
        }
    }
    picture(cache.get(&key), size)
}

/// Who is on screen right now, as anima's config has it.
fn applied() -> Vec<String> {
    let doc = config::read();
    if !config::boolean(&doc, "character", "enabled", true) {
        return Vec::new();
    }
    let source = config::text(&doc, "character", "source", "");
    if source.is_empty() {
        return Vec::new();
    }
    let extra: Vec<String> = doc
        .get("character")
        .and_then(|c| c.get("extra"))
        .and_then(|e| e.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    std::iter::once(source).chain(extra).collect()
}

/// Send the line-up to anima; it changes in place, no restart.
fn apply(r: &Rc<Roster>) {
    let on = r.ui.data.borrow().enabled();
    let before = applied();
    if on == before {
        refresh(r);
        return;
    }
    // whoever is first in anima's list stays first while it's still in, so
    // nothing has to start over; only turning everyone off or on does
    let mut on = on;
    if let Some(k) = before.first().and_then(|f| on.iter().position(|p| p == f)) {
        let first = on.remove(k);
        on.insert(0, first);
    }
    if on == before {
        refresh(r);
        return;
    }
    let restart = on.is_empty() || before.is_empty();
    config::set_team(&on);
    if r.ui.is_on() {
        if restart {
            daemon::restart();
        } else {
            daemon::reload();
        }
    }
    r.ui.say("Applied");
    refresh(r);
}

/// Change the line-up being edited; nothing reaches the screen until Apply.
fn edit(r: &Rc<Roster>, f: impl FnOnce(&mut team::Team) + 'static) {
    let r = r.clone();
    glib::idle_add_local_once(move || {
        {
            let mut d = r.ui.data.borrow_mut();
            f(&mut d);
            d.save();
        }
        refresh(&r);
    });
}

fn refresh(r: &Rc<Roster>) {
    let members = r.ui.data.borrow().team.clone();
    let on = members.iter().filter(|m| m.on).count();
    r.summary.set_label(&format!("{} characters · {on} on", members.len()));
    let dirty = r.ui.data.borrow().enabled() != applied();
    r.apply.set_sensitive(dirty);
    r.pending.set_visible(dirty);
    let same_order = {
        let rows = r.card_rows.borrow();
        !members.is_empty() && rows.len() == members.len() && rows.iter().zip(&members).all(|((p, _, _), m)| p == &m.path)
    };
    if same_order {
        let rows = r.card_rows.borrow();
        for ((_, frame, sw), m) in rows.iter().zip(&members) {
            if m.on {
                frame.remove_css_class("off");
            } else {
                frame.add_css_class("off");
            }
            if sw.is_active() != m.on {
                sw.set_active(m.on);
            }
        }
    } else {
        clear_flow(&r.cards);
        let mut next_rows = Vec::with_capacity(members.len());
        for (i, m) in members.iter().enumerate() {
            let (w, frame, sw) = card(r, i, m);
            r.cards.append(&w);
            next_rows.push((m.path.clone(), frame, sw));
        }
        *r.card_rows.borrow_mut() = next_rows;
        if members.is_empty() {
            r.cards.append(&small("Nobody yet: pick characters on the right."));
        }
    }
    fill_lineups(r);
}

fn card(r: &Rc<Roster>, i: usize, m: &team::Member) -> (gtk::Widget, gtk::Box, gtk::Switch) {
    let pic = thumb(&r.ui, &m.path, 92);
    pic.set_halign(gtk::Align::Center);
    let name = gtk::Label::builder().label(name_of(&m.path)).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(11).build();
    name.add_css_class("caption-heading");
    let sw = gtk::Switch::builder().active(m.on).valign(gtk::Align::Center).tooltip_text("On screen").build();
    let rr = r.clone();
    sw.connect_active_notify(move |w| {
        let on = w.is_active();
        edit(&rr, move |d| {
            if let Some(m) = d.team.get_mut(i) {
                m.on = on;
            }
        });
    });
    let stage = gtk::Overlay::builder().child(&pic).build();
    sw.set_halign(gtk::Align::Center);
    let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    col.append(&stage);
    col.append(&name);
    col.append(&sw);
    let frame = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(128).tooltip_text("Right-click for status and friendships").build();
    frame.add_css_class("char-card");
    if !m.on {
        frame.add_css_class("off");
    }
    frame.append(&col);

    // right-click: the small menu on the card where it was clicked; what it
    // opens comes up in the middle of the window
    let menu = gtk::Popover::new();
    menu.set_parent(&frame);
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let mm = menu.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            mm.popdown();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    menu.add_controller(keys);
    let click = gtk::GestureClick::builder().button(3).build();
    let (rr, path, card) = (r.clone(), m.path.clone(), frame.clone());
    click.connect_pressed(move |_, _, x, y| {
        let p = Panel::Menu(menu.clone(), card.clone().upcast());
        p.set_child(Some(&choices(&rr, &p, i, &path)));
        menu.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        menu.popup();
    });
    frame.add_controller(click);
    (frame.clone().upcast(), frame, sw)
}

/// The character's right-click panel: the small menu pops up on its card;
/// a bigger view (status, friendships, wardrobe, fitting) opens as a dialog
/// in the middle of the window.
#[derive(Clone)]
enum Panel {
    Menu(gtk::Popover, gtk::Widget),
    View(adw::Dialog),
}

impl Panel {
    fn set_child(&self, w: Option<&impl IsA<gtk::Widget>>) {
        match self {
            Panel::Menu(p, _) => p.set_child(w),
            Panel::View(d) => {
                let wrapped = w.map(|w| {
                    let b = gtk::Box::builder().margin_top(12).margin_bottom(12).margin_start(12).margin_end(12).build();
                    b.append(w);
                    b
                });
                d.set_child(wrapped.as_ref());
            }
        }
    }

    fn popdown(&self) {
        match self {
            Panel::Menu(p, _) => p.popdown(),
            Panel::View(d) => {
                d.close();
            }
        }
    }

    /// A bigger view: from the menu, in a dialog in the middle; in the dialog, in its place.
    fn open(&self, make: impl FnOnce(&Panel) -> gtk::Widget) {
        match self {
            Panel::Menu(p, parent) => {
                p.popdown();
                let d = adw::Dialog::builder().follows_content_size(true).build();
                // Esc closes it, wherever the focus is inside
                let keys = gtk::EventControllerKey::new();
                keys.set_propagation_phase(gtk::PropagationPhase::Capture);
                let dd = d.clone();
                keys.connect_key_pressed(move |_, key, _, _| {
                    if key == gdk::Key::Escape {
                        dd.close();
                        return glib::Propagation::Stop;
                    }
                    glib::Propagation::Proceed
                });
                d.add_controller(keys);
                let view = Panel::View(d.clone());
                let w = make(&view);
                view.set_child(Some(&w));
                d.present(Some(parent));
            }
            Panel::View(_) => {
                let w = make(self);
                self.set_child(Some(&w));
            }
        }
    }
}

fn menu_button(label: &str, icon: &str) -> gtk::Button {
    let row = gtk::Box::builder().spacing(8).build();
    row.append(&gtk::Image::from_icon_name(icon));
    row.append(&gtk::Label::new(Some(label)));
    let b = gtk::Button::builder().child(&row).build();
    b.add_css_class("flat");
    b
}

fn choices(r: &Rc<Roster>, pop: &Panel, i: usize, path: &str) -> gtk::Widget {
    let name = name_of(path);
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
    let title = gtk::Label::builder().label(&name).xalign(0.0).margin_start(8).margin_bottom(4).build();
    title.add_css_class("heading");
    list.append(&title);
    let status = menu_button("Status", "dialog-information-symbolic");
    let (rr, p, n) = (r.clone(), pop.clone(), name.clone());
    status.connect_clicked(move |_| p.open(|q| status_view(&rr, q, i, &n)));
    list.append(&status);
    let friends = menu_button("Friendships", "system-users-symbolic");
    let (rr, p, n) = (r.clone(), pop.clone(), name.clone());
    friends.connect_clicked(move |_| p.open(|q| friends_view(&rr, q, i, &n)));
    list.append(&friends);
    let wear = menu_button("Wardrobe", "applications-graphics-symbolic");
    let (rr, p, n) = (r.clone(), pop.clone(), name.clone());
    wear.connect_clicked(move |_| p.open(|q| wardrobe_view(&rr, q, i, &n)));
    list.append(&wear);
    let fit = menu_button("Fit accessories", "object-flip-horizontal-symbolic");
    let (rr, p, n, pth) = (r.clone(), pop.clone(), name.clone(), path.to_string());
    fit.connect_clicked(move |_| p.open(|q| fit_view(&rr, q, i, &n, &pth)));
    list.append(&fit);
    list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let remove = menu_button("Remove from line-up", "user-trash-symbolic");
    let (rr, p) = (r.clone(), pop.clone());
    remove.connect_clicked(move |_| {
        p.popdown();
        edit(&rr, move |d| {
            if i < d.team.len() {
                d.team.remove(i);
            }
        });
    });
    list.append(&remove);
    list.upcast()
}

fn back_header(r: &Rc<Roster>, pop: &Panel, i: usize, name: &str, title: &str) -> gtk::Box {
    let head = gtk::Box::builder().spacing(6).margin_bottom(6).build();
    let back = gtk::Button::builder().icon_name("go-previous-symbolic").tooltip_text("Back").build();
    back.add_css_class("flat");
    let (rr, p) = (r.clone(), pop.clone());
    let path = r.ui.data.borrow().team.get(i).map(|m| m.path.clone()).unwrap_or_default();
    back.connect_clicked(move |_| p.set_child(Some(&choices(&rr, &p, i, &path))));
    head.append(&back);
    let t = gtk::Label::builder().label(format!("{name} · {title}")).xalign(0.0).build();
    t.add_css_class("heading");
    head.append(&t);
    head
}

fn eye_fixes_path() -> std::path::PathBuf {
    anima::util::data_dir().join("eyes.json")
}

fn eye_fixes() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(eye_fixes_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_eye_fixes(m: &serde_json::Map<String, serde_json::Value>) {
    if let Ok(text) = serde_json::to_string_pretty(m) {
        let _ = anima::util::atomic_write(&eye_fixes_path(), text.as_bytes());
    }
}

fn wardrobe_all() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(anima::render::character::wear::wardrobe_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// What the character wears: a switch for each thing there is. The weather
/// and Christmas put on their own things on top of these.
fn fits_all() -> anima::render::character::wear::Fits {
    std::fs::read_to_string(anima::render::character::wear::fit_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_fits(f: &anima::render::character::wear::Fits) {
    if let Ok(text) = serde_json::to_string_pretty(f) {
        let _ = anima::util::atomic_write(&anima::render::character::wear::fit_path(), text.as_bytes());
    }
}

/// The sprite as a cairo surface, downscaled once on load and premultiplied
/// so dragging and zooming stay smooth.
fn sprite_surface(path: &str) -> Option<(gtk::cairo::ImageSurface, image::RgbaImage)> {
    let raw = image::open(path).ok()?.to_rgba8();
    let (w, h) = raw.dimensions();
    let img = if w.max(h) > 512 {
        let scale = 512.0 / w.max(h) as f32;
        let nw = ((w as f32 * scale).round() as u32).max(1);
        let nh = ((h as f32 * scale).round() as u32).max(1);
        image::imageops::resize(&raw, nw, nh, image::imageops::FilterType::Triangle)
    } else {
        raw
    };
    let (w, h) = img.dimensions();
    let mut surf = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w as i32, h as i32).ok()?;
    let stride = surf.stride() as usize;
    {
        let mut data = surf.data().ok()?;
        for (x, y, p) in img.enumerate_pixels() {
            let a = p[3] as u32;
            let o = y as usize * stride + x as usize * 4;
            data[o] = (p[2] as u32 * a / 255) as u8;
            data[o + 1] = (p[1] as u32 * a / 255) as u8;
            data[o + 2] = (p[0] as u32 * a / 255) as u8;
            data[o + 3] = a as u8;
        }
    }
    Some((surf, img))
}

/// Fitting what it wears by hand: each accessory is a box on the picture;
/// drag it to move it, drag its corner or scroll to size it. Saved as it's
/// done, and shown on the desktop at once.
fn fit_view(r: &Rc<Roster>, pop: &Panel, i: usize, name: &str, path: &str) -> gtk::Widget {
    fit_pose(r, pop, i, name, path, false)
}

fn fit_pose(r: &Rc<Roster>, pop: &Panel, i: usize, name: &str, path: &str, happy: bool) -> gtk::Widget {
    use anima::render::character::wear;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    #[derive(Clone)]
    struct Slot {
        item: wear::Item,
        fit_id: String,
        label: String,
        side: Option<f32>,
    }

    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).width_request(780).build();
    let top_row = gtk::Box::builder().spacing(12).build();
    let header = back_header(r, pop, i, name, "Fit accessories");
    header.set_hexpand(true);
    top_row.append(&header);

    // calm and happy are pictures of their own: positions can differ, size stays synced
    let poses = gtk::Box::builder().spacing(0).valign(gtk::Align::Center).build();
    poses.add_css_class("linked");
    let calm_b = gtk::ToggleButton::builder().label("Calm").active(!happy).build();
    let happy_b = gtk::ToggleButton::builder().label("Happy").active(happy).group(&calm_b).build();
    let happy_pic = wear::happy_path(path).filter(|p| std::path::Path::new(p).is_file());
    happy_b.set_sensitive(happy_pic.is_some());
    poses.append(&calm_b);
    poses.append(&happy_b);
    top_row.append(&poses);
    b.append(&top_row);

    // what it wears; paired items (shoes, etc.) split into Left and Right slots
    let on: Vec<String> = wardrobe_all().get(name).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let raw_items: Vec<wear::Item> = wear::catalog().into_iter().filter(|it| on.contains(&it.id)).collect();
    let mut slots: Vec<Slot> = Vec::new();
    for it in raw_items {
        if wear::pair_of(&it) > 0.0 {
            slots.push(Slot {
                fit_id: wear::side_id(&it.id, -1.0),
                label: format!("{} (Left)", it.label),
                side: Some(-1.0),
                item: it.clone(),
            });
            slots.push(Slot {
                fit_id: wear::side_id(&it.id, 1.0),
                label: format!("{} (Right)", it.label),
                side: Some(1.0),
                item: it,
            });
        } else {
            slots.push(Slot {
                fit_id: it.id.clone(),
                label: it.label.clone(),
                side: None,
                item: it,
            });
        }
    }

    let calm_path = path;
    let draw_path = if happy { happy_pic.as_deref().unwrap_or(path) } else { path };
    let Some((surf, img)) = sprite_surface(draw_path) else {
        b.append(&gtk::Label::new(Some("Couldn't read its picture")));
        return b.upcast();
    };
    let found = anima::render::anchors::find(&img);
    let calm_anchors = if happy {
        sprite_surface(calm_path).and_then(|(_, ci)| anima::render::anchors::find(&ci))
    } else {
        found
    };
    let anchors = match (happy, calm_anchors, found) {
        (true, Some(c), Some(h)) => wear::happy_anchors(&c, h),
        (_, _, Some(a)) => a,
        _ => {
            b.append(&gtk::Label::new(Some("Couldn't find its head on the picture")));
            return b.upcast();
        }
    };

    let faces_right = calm_path.contains("_0010.") || calm_path.contains("_0011.");
    let fits: Rc<RefCell<wear::Fits>> = Rc::new(RefCell::new(fits_all()));
    let pick: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let only_sel_box: Rc<Cell<bool>> = Rc::new(Cell::new(true));
    let link_pair: Rc<Cell<bool>> = Rc::new(Cell::new(true));
    let dirty_save: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    let save_deb = crate::pages::Debounce::default();
    let slots = Rc::new(slots);
    let who = name.to_string();
    let (iw, ih) = (img.width() as f64, img.height() as f64);
    let place = move |w: f64, h: f64| {
        let s = (w / iw).min(h / ih);
        (s, (w - iw * s) * 0.5, (h - ih * s) * 0.5)
    };

    let save_now = {
        let (fits, dirty_save) = (fits.clone(), dirty_save.clone());
        Rc::new(move || {
            dirty_save.set(false);
            save_fits(&fits.borrow());
        })
    };
    let save_soon = {
        let (fits, dirty_save, save_deb) = (fits.clone(), dirty_save.clone(), save_deb.clone());
        Rc::new(move || {
            dirty_save.set(true);
            let (fits, dirty_save) = (fits.clone(), dirty_save.clone());
            save_deb.call(180, move || {
                dirty_save.set(false);
                save_fits(&fits.borrow());
            });
        })
    };
    {
        let (dirty_save, save_now) = (dirty_save.clone(), save_now.clone());
        b.connect_destroy(move |_| {
            if dirty_save.get() {
                save_now();
            }
        });
    }
    for (btn, want) in [(calm_b.clone(), false), (happy_b.clone(), true)] {
        let (rr, p, n, pth, save_now, dirty_save) = (r.clone(), pop.clone(), name.to_string(), calm_path.to_string(), save_now.clone(), dirty_save.clone());
        btn.connect_toggled(move |t| {
            if t.is_active() && want != happy {
                if dirty_save.get() {
                    save_now();
                }
                p.set_child(Some(&fit_pose(&rr, &p, i, &n, &pth, want)));
            }
        });
    }

    let fit_of_slot = {
        let (fits, who) = (fits.clone(), who.clone());
        Rc::new(move |sl: &Slot| -> [f32; 4] {
            let f = fits.borrow();
            let mine = f.get(&who);
            if let Some(side) = sl.side {
                let (opt, ind) = wear::fit_of_side(mine, &sl.item.id, side, happy);
                let val = opt.unwrap_or([0.0, 0.0, 1.0, 1.0]);
                if ind { val } else { [val[0] * side, val[1], val[2], val[3]] }
            } else {
                mine.and_then(|m| m.get(&wear::fit_key(&sl.fit_id, happy)).or_else(|| m.get(&sl.fit_id)))
                    .copied()
                    .unwrap_or([0.0, 0.0, 1.0, 1.0])
            }
        })
    };

    let turn_of_slot = {
        let (fits, who) = (fits.clone(), who.clone());
        Rc::new(move |sl: &Slot| -> (f32, bool, bool) {
            let f = fits.borrow();
            let mine = f.get(&who);
            if let Some(side) = sl.side {
                let (t, m) = wear::turn_of_side(mine, &sl.item.id, side, happy);
                let b = wear::behind_of_side(mine, &sl.item.id, side, happy, sl.item.behind);
                (t, m, b)
            } else {
                let (t, m) = wear::turn_of(mine, &sl.fit_id, happy);
                let b = wear::behind_of(mine, &sl.fit_id, happy, sl.item.behind);
                (t, m, b)
            }
        })
    };

    let anchor_of_slot = {
        let (fits, who) = (fits.clone(), who.clone());
        Rc::new(move |sl: &Slot| -> wear::Anchor {
            let f = fits.borrow();
            let mine = f.get(&who);
            if let Some(side) = sl.side {
                wear::anchor_of_side(mine, &sl.item.id, side, happy, sl.item.anchor)
            } else {
                wear::anchor_of(mine, &sl.fit_id, happy, sl.item.anchor)
            }
        })
    };

    let anchor_idx_of_slot = {
        let (fits, who) = (fits.clone(), who.clone());
        Rc::new(move |sl: &Slot| -> u32 {
            let f = fits.borrow();
            let mine = f.get(&who);
            let idx = wear::meta_raw(mine, &sl.fit_id, happy)[0].round() as usize;
            if idx >= 1 && idx <= wear::Anchor::ALL.len() { idx as u32 } else { 0 }
        })
    };

    let order_of_slot = {
        let (fits, who) = (fits.clone(), who.clone());
        Rc::new(move |sl: &Slot| -> f32 {
            let f = fits.borrow();
            let mine = f.get(&who);
            if let Some(side) = sl.side {
                wear::order_of_side(mine, &sl.item.id, side, happy)
            } else {
                wear::order_of(mine, &sl.fit_id, happy)
            }
        })
    };

    let group_order = {
        let (slots, turn_of_slot, order_of_slot) = (slots.clone(), turn_of_slot.clone(), order_of_slot.clone());
        Rc::new(move |behind: bool| -> Vec<usize> {
            let mut ks: Vec<usize> = (0..slots.len()).filter(|&k| turn_of_slot(&slots[k]).2 == behind).collect();
            ks.sort_by(|&a, &b| order_of_slot(&slots[a]).total_cmp(&order_of_slot(&slots[b])).then(a.cmp(&b)));
            ks
        })
    };

    let behind_override_of_slot = {
        let (fits, who) = (fits.clone(), who.clone());
        Rc::new(move |sl: &Slot| -> f32 {
            let f = fits.borrow();
            let mine = f.get(&who);
            let raw = wear::turn_raw(mine, &sl.fit_id, happy);
            if raw[2].abs() > 0.5 {
                raw[2]
            } else if sl.side.is_some() {
                wear::turn_raw(mine, &sl.item.id, happy)[2]
            } else {
                0.0
            }
        })
    };

    let rect_of_slot = {
        let (fit_of_slot, anchor_of_slot) = (fit_of_slot.clone(), anchor_of_slot.clone());
        Rc::new(move |sl: &Slot| -> [f64; 4] {
            let f = fit_of_slot(sl);
            let mut base = sl.item.clone();
            base.anchor = anchor_of_slot(sl);
            let (it, fit) = if let Some(side) = sl.side {
                wear::pair_copy_side(&base, side, Some(f), true)
            } else {
                (base, Some(f))
            };
            let [l, t, w, h] = wear::layout(&anchors, &it, faces_right, fit);
            [l as f64 * iw, t as f64 * ih, w as f64 * iw, h as f64 * ih]
        })
    };

    let area = gtk::DrawingArea::builder().content_width(400).content_height(500).hexpand(true).vexpand(true).build();
    let pictures: Vec<Option<(gtk::cairo::ImageSurface, f64, f64)>> = slots
        .iter()
        .map(|sl| {
            let p = wear::picture_for(&sl.item, &who)?;
            let (s, img) = sprite_surface(p.to_str()?)?;
            Some((s, img.width() as f64, img.height() as f64))
        })
        .collect();

    {
        let (pick, slots, rect_of_slot, turn_of_slot, group_order, only_sel_box) = (pick.clone(), slots.clone(), rect_of_slot.clone(), turn_of_slot.clone(), group_order.clone(), only_sel_box.clone());
        area.set_draw_func(move |_, cr, w, h| {
            let (s, ox, oy) = place(w as f64, h as f64);
            cr.set_source_rgb(0.16, 0.16, 0.18);
            let _ = cr.paint();

            let draw_slot_pic = |k: usize| {
                let sl = &slots[k];
                let [rx, ry, rw, rh] = rect_of_slot(sl);
                let (x, y, rw, rh) = (ox + rx * s, oy + ry * s, rw * s, rh * s);
                let (turn, mirror, _) = turn_of_slot(sl);
                let _ = cr.save();
                cr.translate(x + rw * 0.5, y + rh * 0.5);
                cr.rotate(-turn as f64);
                cr.translate(-(x + rw * 0.5), -(y + rh * 0.5));
                if let Some((pic, pw, ph)) = &pictures[k] {
                    let (dw, dh) = (rw, rw * ph / pw);
                    let _ = cr.save();
                    cr.translate(x, y + rh * 0.5 - dh * 0.5);
                    let paired = sl.side.is_some_and(wear::pair_mirrored);
                    if mirror ^ paired {
                        cr.translate(dw, 0.0);
                        cr.scale(-1.0, 1.0);
                    }
                    cr.scale(dw / pw, dh / ph);
                    let _ = cr.set_source_surface(pic, 0.0, 0.0);
                    let _ = cr.paint();
                    let _ = cr.restore();
                } else {
                    cr.set_source_rgba(0.6, 0.75, 1.0, 0.22);
                    cr.rectangle(x, y, rw, rh);
                    let _ = cr.fill();
                }
                let _ = cr.restore();
            };

            // 1. Behind-character items sorted by draw order
            for k in group_order(true) {
                draw_slot_pic(k);
            }

            // 2. Character sprite
            let _ = cr.save();
            cr.translate(ox, oy);
            cr.scale(s, s);
            let _ = cr.set_source_surface(&surf, 0.0, 0.0);
            let _ = cr.paint();
            let _ = cr.restore();

            // 3. In-front items sorted by draw order
            for k in group_order(false) {
                draw_slot_pic(k);
            }

            // 4. Selection boxes & handles on top
            for k in 0..slots.len() {
                let chosen = k == pick.get();
                if only_sel_box.get() && !chosen {
                    continue;
                }
                let sl = &slots[k];
                let [rx, ry, rw, rh] = rect_of_slot(sl);
                let (x, y, rw, rh) = (ox + rx * s, oy + ry * s, rw * s, rh * s);
                let (turn, _, _) = turn_of_slot(sl);
                let _ = cr.save();
                cr.translate(x + rw * 0.5, y + rh * 0.5);
                cr.rotate(-turn as f64);
                cr.translate(-(x + rw * 0.5), -(y + rh * 0.5));
                if chosen {
                    cr.set_source_rgb(1.0, 0.45, 0.65);
                    cr.set_line_width(2.0);
                } else {
                    cr.set_source_rgba(0.85, 0.85, 0.9, 0.45);
                    cr.set_line_width(1.0);
                }
                cr.rectangle(x, y, rw, rh);
                let _ = cr.stroke();
                if chosen {
                    for (cx, cy) in [(x, y), (x + rw, y), (x, y + rh), (x + rw, y + rh)] {
                        cr.rectangle(cx - 5.0, cy - 5.0, 10.0, 10.0);
                        let _ = cr.fill();
                    }
                    cr.set_font_size(11.0);
                    cr.move_to(x + 2.0, y - 5.0);
                    let _ = cr.show_text(&sl.label);
                }
                let _ = cr.restore();
            }
        });
    }

    let apply_fit = {
        let (fits, who, link_pair) = (fits.clone(), who.clone(), link_pair.clone());
        let sync_other_pose = move |mine: &mut HashMap<String, [f32; 4]>, id: &str, sw: f32, sh: f32| {
            let other_k = wear::fit_key(id, !happy);
            if let Some(cur) = mine.get_mut(&other_k) {
                cur[2] = sw;
                cur[3] = sh;
            } else if happy {
                // editing Happy when Calm had no saved fit yet: create Calm with same size
                mine.insert(other_k, [0.0, 0.0, sw, sh]);
            }
        };
        Rc::new(move |sl: &Slot, f: [f32; 4], sync_size: bool| {
            let mut all = fits.borrow_mut();
            let mine = all.entry(who.clone()).or_default();
            mine.insert(wear::fit_key(&sl.fit_id, happy), f);
            if sync_size {
                sync_other_pose(mine, &sl.fit_id, f[2], f[3]);
                if link_pair.get() {
                    if let Some(side) = sl.side {
                        let other_id = wear::side_id(&sl.item.id, -side);
                        let (opt, ind) = wear::fit_of_side(Some(mine), &sl.item.id, -side, happy);
                        let mut base = opt.unwrap_or([0.0, 0.0, 1.0, 1.0]);
                        if !ind {
                            base[0] *= -side;
                        }
                        base[2] = f[2];
                        base[3] = f[3];
                        mine.insert(wear::fit_key(&other_id, happy), base);
                        sync_other_pose(mine, &other_id, f[2], f[3]);
                    }
                }
            }
        })
    };

    // Right-hand controls panel
    let right = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(7).width_request(340).build();

    // Item selector list
    let list_box = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Single).build();
    list_box.add_css_class("boxed-list");
    let mut layer_badges: Vec<gtk::Label> = Vec::new();
    for sl in slots.iter() {
        let row_box = gtk::Box::builder().spacing(8).margin_top(4).margin_bottom(4).margin_start(8).margin_end(8).build();
        let lbl = gtk::Label::builder().label(&sl.label).xalign(0.0).hexpand(true).build();
        let badge = gtk::Label::new(Some(""));
        badge.add_css_class("dim-label");
        badge.add_css_class("caption");
        row_box.append(&lbl);
        row_box.append(&badge);
        list_box.append(&row_box);
        layer_badges.push(badge);
    }
    let layer_badges = Rc::new(layer_badges);
    let list_scroll = gtk::ScrolledWindow::builder().child(&list_box).max_content_height(140).propagate_natural_height(true).build();
    right.append(&list_scroll);

    // Body part selector (Default, Head, Face, Neck, Body, Arm, Back, Hand, Off-hand, Feet)
    let mut part_labels: Vec<&str> = vec!["Default (item)"];
    for a in wear::Anchor::ALL {
        part_labels.push(a.label());
    }
    let part_drop = gtk::DropDown::from_strings(&part_labels);
    part_drop.set_hexpand(true);
    let part_row = gtk::Box::builder().spacing(8).build();
    let part_lbl = gtk::Label::builder().label("Part").width_chars(6).xalign(0.0).build();
    part_lbl.add_css_class("dim-label");
    part_row.append(&part_lbl);
    part_row.append(&part_drop);
    right.append(&part_row);

    // Layer side + draw order controls
    let front_btn = gtk::ToggleButton::builder().label("In front").hexpand(true).build();
    let behind_btn = gtk::ToggleButton::builder().label("Behind").group(&front_btn).hexpand(true).build();
    let layer_bar = gtk::Box::builder().spacing(0).hexpand(true).build();
    layer_bar.add_css_class("linked");
    layer_bar.append(&front_btn);
    layer_bar.append(&behind_btn);
    let order_down = gtk::Button::builder().icon_name("go-down-symbolic").tooltip_text("Draw earlier (underneath)").build();
    let order_up = gtk::Button::builder().icon_name("go-up-symbolic").tooltip_text("Draw later (on top)").build();
    let order_bar = gtk::Box::builder().spacing(0).build();
    order_bar.add_css_class("linked");
    order_bar.append(&order_down);
    order_bar.append(&order_up);
    let layer_row = gtk::Box::builder().spacing(6).build();
    layer_row.append(&layer_bar);
    layer_row.append(&order_bar);
    right.append(&layer_row);

    let make_scale = |min: f64, max: f64, step: f64| {
        let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
        s.set_hexpand(true);
        s.set_draw_value(true);
        s.set_value_pos(gtk::PositionType::Right);
        s
    };
    let size_scale = make_scale(0.15, 4.0, 0.02);
    let width_scale = make_scale(0.15, 4.0, 0.02);
    let height_scale = make_scale(0.15, 4.0, 0.02);
    let angle_scale = make_scale(-180.0, 180.0, 1.0);
    angle_scale.set_digits(0);

    let labeled = |title: &str, w: &gtk::Scale| {
        let row = gtk::Box::builder().spacing(8).build();
        let l = gtk::Label::builder().label(title).width_chars(6).xalign(0.0).build();
        l.add_css_class("dim-label");
        row.append(&l);
        row.append(w);
        row
    };
    right.append(&labeled("Scale", &size_scale));
    right.append(&labeled("Width", &width_scale));
    right.append(&labeled("Height", &height_scale));
    right.append(&labeled("Angle", &angle_scale));

    // Fine nudge & rotation buttons
    let nudge_row = gtk::Box::builder().spacing(4).halign(gtk::Align::Center).build();
    let left_b = gtk::Button::builder().icon_name("go-previous-symbolic").tooltip_text("Move left").build();
    let up_b = gtk::Button::builder().icon_name("go-up-symbolic").tooltip_text("Move up").build();
    let down_b = gtk::Button::builder().icon_name("go-down-symbolic").tooltip_text("Move down").build();
    let right_b = gtk::Button::builder().icon_name("go-next-symbolic").tooltip_text("Move right").build();
    let smaller_b = gtk::Button::builder().icon_name("zoom-out-symbolic").tooltip_text("Smaller").build();
    let larger_b = gtk::Button::builder().icon_name("zoom-in-symbolic").tooltip_text("Larger").build();
    for btn in [&left_b, &up_b, &down_b, &right_b, &smaller_b, &larger_b] {
        nudge_row.append(btn);
    }
    right.append(&nudge_row);

    let turns = gtk::Box::builder().spacing(4).halign(gtk::Align::Center).build();
    let rot_l = gtk::Button::builder().icon_name("object-rotate-left-symbolic").tooltip_text("Turn left 5°").build();
    let rot_r = gtk::Button::builder().icon_name("object-rotate-right-symbolic").tooltip_text("Turn right 5°").build();
    let flip_b = gtk::Button::builder().icon_name("object-flip-horizontal-symbolic").tooltip_text("Mirror").build();
    let straight = gtk::Button::builder().label("0°").tooltip_text("Reset angle & mirror").build();
    for btn in [&rot_l, &rot_r, &flip_b, &straight] {
        turns.append(btn);
    }
    right.append(&turns);

    let only_box_chk = gtk::CheckButton::builder().label("Only show selected box").active(true).build();
    let link_pair_chk = gtk::CheckButton::builder().label("Resize left & right together").active(true).build();
    link_pair_chk.set_visible(slots.iter().any(|sl| sl.side.is_some()));
    right.append(&only_box_chk);
    right.append(&link_pair_chk);

    let action_row = gtk::Box::builder().spacing(6).build();
    let mirror_other = gtk::Button::builder().label("Copy to other side").tooltip_text("Copy size, height and mirrored position to the other shoe").hexpand(true).build();
    let reset = gtk::Button::builder().label("Reset item").hexpand(true).build();
    action_row.append(&mirror_other);
    action_row.append(&reset);
    right.append(&action_row);

    let syncing = Rc::new(Cell::new(false));
    let sync_ui = {
        let (syncing, slots, pick, fit_of_slot, turn_of_slot, anchor_of_slot, anchor_idx_of_slot, group_order, list_box, layer_badges, part_drop, front_btn, behind_btn, order_down, order_up, size_scale, width_scale, height_scale, angle_scale, mirror_other, right) = (
            syncing.clone(),
            slots.clone(),
            pick.clone(),
            fit_of_slot.clone(),
            turn_of_slot.clone(),
            anchor_of_slot.clone(),
            anchor_idx_of_slot.clone(),
            group_order.clone(),
            list_box.clone(),
            layer_badges.clone(),
            part_drop.clone(),
            front_btn.clone(),
            behind_btn.clone(),
            order_down.clone(),
            order_up.clone(),
            size_scale.clone(),
            width_scale.clone(),
            height_scale.clone(),
            angle_scale.clone(),
            mirror_other.clone(),
            right.clone(),
        );
        Rc::new(move || {
            syncing.set(true);
            let front_group = group_order(false);
            let behind_group = group_order(true);
            for (k, sl) in slots.iter().enumerate() {
                if let Some(b) = layer_badges.get(k) {
                    let behind = turn_of_slot(sl).2;
                    let group = if behind { &behind_group } else { &front_group };
                    let rank = group.iter().position(|&idx| idx == k).map_or(1, |p| p + 1);
                    let layer = if behind { "Behind" } else { "Front" };
                    b.set_text(&format!("{} · {layer} #{rank}", anchor_of_slot(sl).label()));
                }
            }
            let k = pick.get();
            if let Some(row) = list_box.row_at_index(k as i32) {
                list_box.select_row(Some(&row));
            }
            if let Some(sl) = slots.get(k) {
                let f = fit_of_slot(sl);
                let (t, _, behind) = turn_of_slot(sl);
                let group = if behind { &behind_group } else { &front_group };
                let pos = group.iter().position(|&idx| idx == k).unwrap_or(0);
                part_drop.set_selected(anchor_idx_of_slot(sl));
                front_btn.set_active(!behind);
                behind_btn.set_active(behind);
                order_down.set_sensitive(pos > 0);
                order_up.set_sensitive(pos + 1 < group.len());
                size_scale.set_value(((f[2] + f[3]) * 0.5) as f64);
                width_scale.set_value(f[2] as f64);
                height_scale.set_value(f[3] as f64);
                angle_scale.set_value(t.to_degrees() as f64);
                mirror_other.set_visible(sl.side.is_some());
                right.set_sensitive(true);
            }
            syncing.set(false);
        })
    };
    sync_ui();

    // Drag & wheel on the canvas
    let drag = gtk::GestureDrag::new();
    // (initial_fit, corner_sign: Some((sx, sy)) where sx/sy in {-1.0, 1.0})
    let start: Rc<Cell<Option<([f32; 4], Option<(f32, f32)>)>>> = Rc::new(Cell::new(None));
    {
        let (pick, slots, rect_of_slot, start, fit_of_slot, area2, sync_ui) = (pick.clone(), slots.clone(), rect_of_slot.clone(), start.clone(), fit_of_slot.clone(), area.clone(), sync_ui.clone());
        drag.connect_drag_begin(move |g, x, y| {
            let Some(w) = g.widget() else { return };
            if slots.is_empty() {
                return;
            }
            let (s, ox, oy) = place(w.width() as f64, w.height() as f64);
            let (px, py) = ((x - ox) / s, (y - oy) / s);
            let cur = pick.get();
            let [cx, cy, cw, ch] = rect_of_slot(&slots[cur]);
            let corner = [
                (cx, cy, (-1.0f32, -1.0f32)),
                (cx + cw, cy, (1.0, -1.0)),
                (cx, cy + ch, (-1.0, 1.0)),
                (cx + cw, cy + ch, (1.0, 1.0)),
            ]
            .into_iter()
            .find(|(hx, hy, _)| (px - *hx).abs() * s < 12.0 && (py - *hy).abs() * s < 12.0)
            .map(|(_, _, sign)| sign);

            if corner.is_none() {
                let inside_cur = px >= cx && px <= cx + cw && py >= cy && py <= cy + ch;
                if !inside_cur {
                    let mut hit: Option<(usize, f64)> = None;
                    for (k, sl) in slots.iter().enumerate() {
                        let [rx, ry, rw, rh] = rect_of_slot(sl);
                        if px >= rx && px <= rx + rw && py >= ry && py <= ry + rh && hit.is_none_or(|h| rw * rh < h.1) {
                            hit = Some((k, rw * rh));
                        }
                    }
                    if let Some((k, _)) = hit {
                        pick.set(k);
                        sync_ui();
                    }
                }
            }
            let cur_sl = &slots[pick.get()];
            start.set(Some((fit_of_slot(cur_sl), corner)));
            area2.queue_draw();
        });
    }
    {
        let (pick, slots, start, apply_fit, area2, sync_ui, rect_of_slot, save_soon) = (pick.clone(), slots.clone(), start.clone(), apply_fit.clone(), area.clone(), sync_ui.clone(), rect_of_slot.clone(), save_soon.clone());
        drag.connect_drag_update(move |g, dx, dy| {
            let Some(w) = g.widget() else { return };
            let Some((f, corner)) = start.get() else { return };
            let sl = &slots[pick.get()];
            let (s, _, _) = place(w.width() as f64, w.height() as f64);
            let (du, dv) = ((dx / s / iw) as f32, (dy / s / ih) as f32);
            let (next, resized) = if let Some((sx, sy)) = corner {
                let base = rect_of_slot(sl);
                let bw = (base[2] / iw).max(0.01) as f32;
                let bh = (base[3] / ih).max(0.01) as f32;
                let k = ((sx * du / bw + sy * dv / bh) * 0.5).max(-0.9);
                ([f[0], f[1], (f[2] * (1.0 + k)).clamp(0.1, 6.0), (f[3] * (1.0 + k)).clamp(0.1, 6.0)], true)
            } else {
                ([f[0] + du, f[1] + dv, f[2], f[3]], false)
            };
            apply_fit(sl, next, resized);
            area2.queue_draw();
            sync_ui();
            save_soon();
        });
    }
    {
        let (save_now, start, u) = (save_now.clone(), start.clone(), r.ui.clone());
        drag.connect_drag_end(move |_, _, _| {
            if start.take().is_some() {
                save_now();
                u.say("Fitted");
            }
        });
    }
    area.add_controller(drag);

    let set_turn_state = {
        let (fits, who, save_soon, area2, pick, slots, turn_of_slot, behind_override_of_slot, sync_ui) = (
            fits.clone(),
            who.clone(),
            save_soon.clone(),
            area.clone(),
            pick.clone(),
            slots.clone(),
            turn_of_slot.clone(),
            behind_override_of_slot.clone(),
            sync_ui.clone(),
        );
        Rc::new(move |update: &dyn Fn(f32, bool, f32) -> (f32, bool, f32)| {
            let Some(sl) = slots.get(pick.get()) else { return };
            let (t, m, _) = turn_of_slot(sl);
            let b_raw = behind_override_of_slot(sl);
            let (t, m, b_raw) = update(t, m, b_raw);
            fits.borrow_mut().entry(who.clone()).or_default().insert(wear::turn_key(&sl.fit_id, happy), [t, if m { 1.0 } else { 0.0 }, b_raw, 0.0]);
            save_soon();
            area2.queue_draw();
            sync_ui();
        })
    };

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    {
        let (pick, slots, apply_fit, fit_of_slot, area2, save_soon, sync_ui) = (pick.clone(), slots.clone(), apply_fit.clone(), fit_of_slot.clone(), area.clone(), save_soon.clone(), sync_ui.clone());
        let st = set_turn_state.clone();
        scroll.connect_scroll(move |ec, _, dy| {
            let Some(sl) = slots.get(pick.get()) else {
                return gtk::glib::Propagation::Proceed;
            };
            if ec.current_event_state().contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                let step = if dy < 0.0 { 5f32 } else { -5.0 };
                st(&|t, m, b| (t + step.to_radians(), m, b));
                return gtk::glib::Propagation::Stop;
            }
            let f = fit_of_slot(sl);
            let k = 1.04f64.powf(-dy.clamp(-3.0, 3.0)) as f32;
            apply_fit(sl, [f[0], f[1], (f[2] * k).clamp(0.1, 6.0), (f[3] * k).clamp(0.1, 6.0)], true);
            area2.queue_draw();
            sync_ui();
            save_soon();
            gtk::glib::Propagation::Stop
        });
    }
    area.add_controller(scroll);

    // Wire right-hand controls
    {
        let (pick, area2, sync_ui) = (pick.clone(), area.clone(), sync_ui.clone());
        list_box.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                let idx = row.index().max(0) as usize;
                if pick.get() != idx {
                    pick.set(idx);
                    area2.queue_draw();
                    sync_ui();
                }
            }
        });
    }
    {
        let (syncing, pick, slots, fits, who, order_of_slot, area2, save_soon, sync_ui) = (
            syncing.clone(),
            pick.clone(),
            slots.clone(),
            fits.clone(),
            who.clone(),
            order_of_slot.clone(),
            area.clone(),
            save_soon.clone(),
            sync_ui.clone(),
        );
        part_drop.connect_selected_notify(move |dd| {
            if syncing.get() {
                return;
            }
            let Some(sl) = slots.get(pick.get()) else { return };
            let ord = order_of_slot(sl);
            let sel = dd.selected() as f32;
            {
                let mut all = fits.borrow_mut();
                let mine = all.entry(who.clone()).or_default();
                mine.insert(wear::meta_key(&sl.fit_id, false), [sel, ord, 0.0, 0.0]);
                mine.insert(wear::meta_key(&sl.fit_id, true), [sel, ord, 0.0, 0.0]);
            }
            save_soon();
            area2.queue_draw();
            sync_ui();
        });
    }
    for (btn, dir) in [(&order_down, -1i32), (&order_up, 1i32)] {
        let (pick, slots, turn_of_slot, group_order, anchor_idx_of_slot, fits, who, area2, save_soon, sync_ui) = (
            pick.clone(),
            slots.clone(),
            turn_of_slot.clone(),
            group_order.clone(),
            anchor_idx_of_slot.clone(),
            fits.clone(),
            who.clone(),
            area.clone(),
            save_soon.clone(),
            sync_ui.clone(),
        );
        btn.connect_clicked(move |_| {
            let k = pick.get();
            let Some(sl) = slots.get(k) else { return };
            let behind = turn_of_slot(sl).2;
            let mut group = group_order(behind);
            let Some(pos) = group.iter().position(|&idx| idx == k) else { return };
            let target = if dir < 0 && pos > 0 {
                pos - 1
            } else if dir > 0 && pos + 1 < group.len() {
                pos + 1
            } else {
                return;
            };
            group.swap(pos, target);
            let anchors_idx: Vec<(usize, f32)> = group.iter().map(|&idx| (idx, anchor_idx_of_slot(&slots[idx]) as f32)).collect();
            {
                let mut all = fits.borrow_mut();
                let mine = all.entry(who.clone()).or_default();
                for (rank, (idx, a_idx)) in anchors_idx.into_iter().enumerate() {
                    let id = &slots[idx].fit_id;
                    let val = [a_idx, rank as f32 + 1.0, 0.0, 0.0];
                    mine.insert(wear::meta_key(id, false), val);
                    mine.insert(wear::meta_key(id, true), val);
                }
            }
            save_soon();
            area2.queue_draw();
            sync_ui();
        });
    }
    for (btn, want_behind) in [(front_btn.clone(), false), (behind_btn.clone(), true)] {
        let (syncing, st) = (syncing.clone(), set_turn_state.clone());
        btn.connect_toggled(move |t| {
            if syncing.get() || !t.is_active() {
                return;
            }
            st(&|turn, m, _| (turn, m, if want_behind { 1.0 } else { -1.0 }));
        });
    }
    {
        let (syncing, pick, slots, fit_of_slot, apply_fit, area2, save_soon, sync_ui) = (syncing.clone(), pick.clone(), slots.clone(), fit_of_slot.clone(), apply_fit.clone(), area.clone(), save_soon.clone(), sync_ui.clone());
        size_scale.connect_value_changed(move |s| {
            if syncing.get() {
                return;
            }
            let Some(sl) = slots.get(pick.get()) else { return };
            let f = fit_of_slot(sl);
            let cur = ((f[2] + f[3]) * 0.5).max(0.05);
            let ratio = s.value() as f32 / cur;
            apply_fit(sl, [f[0], f[1], (f[2] * ratio).clamp(0.1, 6.0), (f[3] * ratio).clamp(0.1, 6.0)], true);
            area2.queue_draw();
            sync_ui();
            save_soon();
        });
    }
    {
        let (syncing, pick, slots, fit_of_slot, apply_fit, area2, save_soon, sync_ui) = (syncing.clone(), pick.clone(), slots.clone(), fit_of_slot.clone(), apply_fit.clone(), area.clone(), save_soon.clone(), sync_ui.clone());
        width_scale.connect_value_changed(move |s| {
            if syncing.get() {
                return;
            }
            let Some(sl) = slots.get(pick.get()) else { return };
            let f = fit_of_slot(sl);
            apply_fit(sl, [f[0], f[1], s.value() as f32, f[3]], true);
            area2.queue_draw();
            sync_ui();
            save_soon();
        });
    }
    {
        let (syncing, pick, slots, fit_of_slot, apply_fit, area2, save_soon, sync_ui) = (syncing.clone(), pick.clone(), slots.clone(), fit_of_slot.clone(), apply_fit.clone(), area.clone(), save_soon.clone(), sync_ui.clone());
        height_scale.connect_value_changed(move |s| {
            if syncing.get() {
                return;
            }
            let Some(sl) = slots.get(pick.get()) else { return };
            let f = fit_of_slot(sl);
            apply_fit(sl, [f[0], f[1], f[2], s.value() as f32], true);
            area2.queue_draw();
            sync_ui();
            save_soon();
        });
    }
    {
        let (syncing, st) = (syncing.clone(), set_turn_state.clone());
        angle_scale.connect_value_changed(move |s| {
            if syncing.get() {
                return;
            }
            let rad = (s.value() as f32).to_radians();
            st(&|_, m, b| (rad, m, b));
        });
    }
    for (btn, dx, dy, ds) in [
        (&left_b, -0.015f32, 0.0, 1.0f32),
        (&right_b, 0.015, 0.0, 1.0),
        (&up_b, 0.0, -0.015, 1.0),
        (&down_b, 0.0, 0.015, 1.0),
        (&smaller_b, 0.0, 0.0, 0.92),
        (&larger_b, 0.0, 0.0, 1.08),
    ] {
        let (pick, slots, fit_of_slot, apply_fit, area2, save_soon, sync_ui) = (pick.clone(), slots.clone(), fit_of_slot.clone(), apply_fit.clone(), area.clone(), save_soon.clone(), sync_ui.clone());
        btn.connect_clicked(move |_| {
            let Some(sl) = slots.get(pick.get()) else { return };
            let f = fit_of_slot(sl);
            apply_fit(sl, [f[0] + dx, f[1] + dy, (f[2] * ds).clamp(0.1, 6.0), (f[3] * ds).clamp(0.1, 6.0)], ds != 1.0);
            area2.queue_draw();
            sync_ui();
            save_soon();
        });
    }
    {
        let st = set_turn_state.clone();
        rot_l.connect_clicked(move |_| st(&|t, m, b| (t + 5f32.to_radians(), m, b)));
    }
    {
        let st = set_turn_state.clone();
        rot_r.connect_clicked(move |_| st(&|t, m, b| (t - 5f32.to_radians(), m, b)));
    }
    {
        let st = set_turn_state.clone();
        flip_b.connect_clicked(move |_| st(&|t, m, b| (-t, !m, b)));
    }
    {
        let st = set_turn_state.clone();
        straight.connect_clicked(move |_| st(&|_, _, b| (0.0, false, b)));
    }
    {
        let (only_sel_box, area2) = (only_sel_box.clone(), area.clone());
        only_box_chk.connect_toggled(move |c| {
            only_sel_box.set(c.is_active());
            area2.queue_draw();
        });
    }
    {
        let link_pair = link_pair.clone();
        link_pair_chk.connect_toggled(move |c| link_pair.set(c.is_active()));
    }
    {
        let (pick, slots, fit_of_slot, fits, who, turn_of_slot, behind_override_of_slot, area2, save_now, sync_ui, u) = (
            pick.clone(),
            slots.clone(),
            fit_of_slot.clone(),
            fits.clone(),
            who.clone(),
            turn_of_slot.clone(),
            behind_override_of_slot.clone(),
            area.clone(),
            save_now.clone(),
            sync_ui.clone(),
            r.ui.clone(),
        );
        mirror_other.connect_clicked(move |_| {
            let Some(sl) = slots.get(pick.get()) else { return };
            let Some(side) = sl.side else { return };
            let other_id = wear::side_id(&sl.item.id, -side);
            let f = fit_of_slot(sl);
            let (t, m, _) = turn_of_slot(sl);
            let b_raw = behind_override_of_slot(sl);
            {
                let mut all = fits.borrow_mut();
                let mine = all.entry(who.clone()).or_default();
                mine.insert(wear::fit_key(&other_id, happy), [-f[0], f[1], f[2], f[3]]);
                let alt_k = wear::fit_key(&other_id, !happy);
                if let Some(cur) = mine.get_mut(&alt_k) {
                    cur[2] = f[2];
                    cur[3] = f[3];
                } else if happy {
                    mine.insert(alt_k, [-f[0], f[1], f[2], f[3]]);
                }
                mine.insert(wear::turn_key(&other_id, happy), [-t, if m { 1.0 } else { 0.0 }, b_raw, 0.0]);
            }
            save_now();
            area2.queue_draw();
            sync_ui();
            u.say("Copied to the other side");
        });
    }
    {
        let (pick, slots, fits, who, area2, save_now, sync_ui) = (pick.clone(), slots.clone(), fits.clone(), who.clone(), area.clone(), save_now.clone(), sync_ui.clone());
        reset.connect_clicked(move |_| {
            let Some(sl) = slots.get(pick.get()) else { return };
            if let Some(m) = fits.borrow_mut().get_mut(&who) {
                m.remove(&wear::fit_key(&sl.fit_id, happy));
                m.remove(&wear::turn_key(&sl.fit_id, happy));
                m.remove(&wear::meta_key(&sl.fit_id, false));
                m.remove(&wear::meta_key(&sl.fit_id, true));
                if sl.side.is_some() {
                    m.remove(&wear::fit_key(&sl.item.id, happy));
                    m.remove(&wear::turn_key(&sl.item.id, happy));
                    m.remove(&wear::meta_key(&sl.item.id, false));
                    m.remove(&wear::meta_key(&sl.item.id, true));
                }
            }
            save_now();
            area2.queue_draw();
            sync_ui();
        });
    }

    let body = gtk::Box::builder().spacing(12).build();
    let frame = gtk::Frame::builder().child(&area).hexpand(true).vexpand(true).build();
    body.append(&frame);
    body.append(&right);
    b.append(&body);

    if on.is_empty() {
        let none = gtk::Label::builder().label("It wears nothing yet: pick things in Wardrobe first.").wrap(true).xalign(0.0).build();
        none.add_css_class("dim-label");
        b.append(&none);
    }
    b.upcast()
}

fn wardrobe_view(r: &Rc<Roster>, pop: &Panel, i: usize, name: &str) -> gtk::Widget {
    use anima::render::character::wear;
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).width_request(280).build();
    b.append(&back_header(r, pop, i, name, "Wardrobe"));
    let hint = gtk::Label::builder()
        .label("Worn at once, on top of umbrellas and snow. Locked things are found while fishing.")
        .wrap(true)
        .xalign(0.0)
        .build();
    hint.add_css_class("dim-label");
    b.append(&hint);
    let on: Vec<String> = wardrobe_all().get(name).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let inv: wear::Inventory = std::fs::read_to_string(wear::inventory_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
    // what it owns first, then the rest, locked till it fishes them up
    let mut all = wear::catalog();
    all.sort_by_key(|it| !wear::owns(&inv, name, &it.id));
    for item in all {
        let owned = wear::owns(&inv, name, &item.id);
        let row = gtk::Box::builder().spacing(8).build();
        let label = gtk::Label::builder().label(if owned { item.label.clone() } else { format!("🔒 {}", item.label) }).xalign(0.0).hexpand(true).build();
        if !owned {
            label.add_css_class("dim-label");
            label.set_tooltip_text(Some("Not found yet: it turns up on a fishing line"));
        }
        row.append(&label);
        let sw = gtk::Switch::builder().active(owned && on.contains(&item.id)).sensitive(owned).valign(gtk::Align::Center).build();
        let (who, id, u) = (name.to_string(), item.id.clone(), r.ui.clone());
        sw.connect_active_notify(move |s| {
            let mut all = wardrobe_all();
            let mut mine: Vec<String> = all.get(&who).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
            mine.retain(|x| x != &id);
            if s.is_active() {
                mine.push(id.clone());
            }
            if mine.is_empty() {
                all.remove(&who);
            } else {
                all.insert(who.clone(), serde_json::json!(mine));
            }
            if let Ok(text) = serde_json::to_string_pretty(&all) {
                let _ = anima::util::atomic_write(&wear::wardrobe_path(), text.as_bytes());
            }
            u.say(if s.is_active() { "Put on" } else { "Taken off" });
        });
        row.append(&sw);
        list.append(&row);
    }
    let scroll = gtk::ScrolledWindow::builder().child(&list).min_content_height(260).max_content_height(360).propagate_natural_height(true).build();
    b.append(&scroll);
    let add = gtk::Button::with_label("Add your own…");
    add.set_tooltip_text(Some("Drop any PNG into this folder, then position, size, and set Front/Behind inside Fit accessories (optional .toml also works)."));
    add.connect_clicked(|_| {
        let dir = wear::accessories_dir();
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
    });
    b.append(&add);
    b.upcast()
}

/// Everything anima knows about the characters, or None if it isn't running.
fn live() -> Option<Vec<serde_json::Value>> {
    let reply = anima::ipc::request(&anima::util::socket_path(), &anima::ipc::Request::CharacterInfo).ok()?;
    reply.data?.as_array().cloned()
}

fn status_view(r: &Rc<Roster>, pop: &Panel, i: usize, name: &str) -> gtk::Widget {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).width_request(260).build();
    b.append(&back_header(r, pop, i, name, "Status"));
    let me = live().and_then(|all| all.into_iter().find(|c| c["name"].as_str() == Some(name)));
    let Some(me) = me else {
        b.append(&small("Not on screen right now (off, or Anima isn't running)."));
        return b.upcast();
    };
    let line = |key: &str, value: String| {
        let row = gtk::Box::builder().spacing(8).build();
        let k = gtk::Label::builder().label(key).xalign(0.0).width_chars(12).build();
        k.add_css_class("dim-label");
        row.append(&k);
        row.append(&gtk::Label::builder().label(value).xalign(0.0).wrap(true).hexpand(true).build());
        row
    };
    b.append(&line("Doing", me["doing"].as_str().unwrap_or("").to_string()));
    let with: Vec<&str> = me["with"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
    if !with.is_empty() {
        b.append(&line("With", with.join(", ")));
    }
    if let Some(ws) = me["workspace"].as_u64() {
        b.append(&line("Workspace", ws.to_string()));
    }
    let energy = me["energy"].as_f64().unwrap_or(0.0);
    let row = gtk::Box::builder().spacing(8).build();
    let k = gtk::Label::builder().label("Social battery").xalign(0.0).width_chars(12).build();
    k.add_css_class("dim-label");
    row.append(&k);
    row.append(&gtk::LevelBar::builder().min_value(0.0).max_value(1.0).value(energy).hexpand(true).valign(gtk::Align::Center).build());
    b.append(&row);
    // what it has got good at
    let skills: Vec<(String, f64)> = me["skills"].as_array().map(|a| a.iter().filter_map(|v| Some((v["game"].as_str()?.to_string(), v["skill"].as_f64()?))).collect()).unwrap_or_default();
    if !skills.is_empty() {
        let grid = gtk::Grid::builder().row_spacing(4).column_spacing(8).margin_top(4).build();
        grid.attach(&small("Getting good at"), 0, 0, 2, 1);
        for (k, (game, v)) in skills.iter().take(6).enumerate() {
            let row = k as i32 + 1;
            let name = gtk::Label::builder().label(game).xalign(0.0).width_chars(14).build();
            name.add_css_class("dim-label");
            grid.attach(&name, 0, row, 1, 1);
            grid.attach(&gtk::LevelBar::builder().min_value(0.0).max_value(1.0).value(*v).hexpand(true).valign(gtk::Align::Center).build(), 1, row, 1, 1);
        }
        b.append(&grid);
    }
    let keeps: Vec<&str> = me["keepsakes"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
    if !keeps.is_empty() {
        // "a star ×3, a rubber duck"
        let mut seen: Vec<(&str, usize)> = Vec::new();
        for k in keeps {
            match seen.iter_mut().find(|s| s.0 == k) {
                Some(s) => s.1 += 1,
                None => seen.push((k, 1)),
            }
        }
        let text = seen.iter().map(|(k, n)| if *n > 1 { format!("{k} ×{n}") } else { k.to_string() }).collect::<Vec<_>>().join(", ");
        b.append(&line("Keepsakes", text));
    }
    let found: Vec<&str> = me["found"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
    if !found.is_empty() {
        b.append(&line("Found", found.join(", ")));
    }
    let played = me["played"].as_f64().unwrap_or(0.0);
    if played >= 0.5 {
        b.append(&line("Games lately", format!("{played:.0}")));
    }
    b.upcast()
}

fn friends_view(r: &Rc<Roster>, pop: &Panel, i: usize, name: &str) -> gtk::Widget {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).width_request(300).build();
    b.append(&back_header(r, pop, i, name, "Friendships"));
    let me = live().and_then(|all| all.into_iter().find(|c| c["name"].as_str() == Some(name)));
    // anima running: the live numbers; otherwise what it last saved
    let bonds: Vec<(String, f64, f64)> = match me {
        Some(me) => me["bonds"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| Some((v["with"].as_str()?.to_string(), v["friend"].as_f64()?, v["rival"].as_f64().unwrap_or(0.0)))).collect())
            .unwrap_or_default(),
        None => saved_bonds(name),
    };
    if bonds.is_empty() {
        b.append(&small("Hasn't really met anyone yet."));
        return b.upcast();
    }
    let grid = gtk::Grid::builder().row_spacing(4).column_spacing(8).build();
    grid.attach(&small("Friends"), 1, 0, 1, 1);
    grid.attach(&small("Rivals"), 2, 0, 1, 1);
    for (k, (other, friend, rival)) in bonds.iter().take(12).enumerate() {
        let row = k as i32 + 1;
        grid.attach(&gtk::Label::builder().label(other).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(14).build(), 0, row, 1, 1);
        grid.attach(&gtk::LevelBar::builder().min_value(0.0).max_value(3.0).value(*friend).width_request(90).valign(gtk::Align::Center).build(), 1, row, 1, 1);
        grid.attach(&gtk::LevelBar::builder().min_value(0.0).max_value(3.0).value(*rival).width_request(60).valign(gtk::Align::Center).build(), 2, row, 1, 1);
    }
    b.append(&grid);
    if bonds.len() > 12 {
        b.append(&small(&format!("and {} more", bonds.len() - 12)));
    }
    b.upcast()
}

fn saved_bonds(name: &str) -> Vec<(String, f64, f64)> {
    let file = config::home().join(".local/share/anima/bonds.json");
    let map: std::collections::HashMap<String, [f64; 2]> = std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut v: Vec<(String, f64, f64)> = map
        .into_iter()
        .filter_map(|(k, v)| {
            let (a, b) = k.split_once('|')?;
            let other = if a == name { b } else if b == name { a } else { return None };
            Some((other.to_string(), v[0], v[1]))
        })
        .collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v
}

fn fill_lineups(r: &Rc<Roster>) {
    clear_box(&r.lineups);
    let saved: Vec<(String, usize)> = r.ui.data.borrow().groups.iter().map(|(k, v)| (k.clone(), v.len())).collect();
    if saved.is_empty() {
        r.lineups.append(&small("No saved line-ups yet."));
    }
    for (name, count) in saved {
        let row = gtk::Box::builder().spacing(4).build();
        let inner = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).margin_top(4).margin_bottom(4).build();
        let head = gtk::Box::builder().spacing(8).build();
        let title = gtk::Label::builder().label(&name).xalign(0.0).hexpand(true).build();
        title.add_css_class("heading");
        head.append(&title);
        head.append(&small(&format!("{count} characters")));
        inner.append(&head);
        let faces = gtk::Box::builder().spacing(2).build();
        let members: Vec<team::Member> = r.ui.data.borrow().groups.get(&name).cloned().unwrap_or_default();
        for m in members.iter().filter(|m| m.on).take(10) {
            faces.append(&thumb(&r.ui, &m.path, 34));
        }
        if members.iter().filter(|m| m.on).count() > 10 {
            faces.append(&small(&format!("+{}", members.iter().filter(|m| m.on).count() - 10)));
        }
        inner.append(&faces);
        let load = gtk::Button::builder().child(&inner).hexpand(true).tooltip_text("Put this line-up in the editor (Apply shows it)").build();
        load.add_css_class("flat");
        let (rr, nm) = (r.clone(), name.clone());
        load.connect_clicked(move |_| {
            let nm = nm.clone();
            edit(&rr, move |d| {
                if let Some(saved) = d.groups.get(&nm).cloned() {
                    d.team = saved;
                }
            });
        });
        row.append(&load);
        let del = gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text("Delete").build();
        del.add_css_class("flat");
        let (rr, nm) = (r.clone(), name.clone());
        del.connect_clicked(move |_| {
            let nm = nm.clone();
            edit(&rr, move |d| {
                d.groups.remove(&nm);
            });
        });
        row.append(&del);
        r.lineups.append(&row);
    }
}

/// The right side: every character; click one to pick an outfit.
fn picker(r: &Rc<Roster>) -> gtk::Widget {
    let search = gtk::SearchEntry::builder().placeholder_text("Search characters").build();
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(8)
        .min_children_per_line(3)
        .row_spacing(2)
        .column_spacing(2)
        .valign(gtk::Align::Start)
        .homogeneous(true)
        .build();
    let dirs: Rc<Vec<PathBuf>> = Rc::new(team::characters());
    let mut pending: Vec<(gtk::Picture, PathBuf)> = Vec::new();
    for d in dirs.iter() {
        let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let pic = gtk::Picture::builder().width_request(52).height_request(52).can_shrink(true).build();
        if let Some(o) = team::outfits(d).into_iter().next() {
            pending.push((pic.clone(), o.calm));
        }
        let label = gtk::Label::builder().label(&name).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(10).build();
        label.add_css_class("caption");
        let inner = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(1).build();
        inner.append(&pic);
        inner.append(&label);
        let pop = gtk::Popover::new();
        let b = gtk::MenuButton::builder().child(&inner).tooltip_text(&name).popover(&pop).build();
        b.add_css_class("flat");
        let (rr, dir) = (r.clone(), d.clone());
        pop.connect_show(move |p| p.set_child(Some(&outfits(&rr, &dir, p))));
        grid.append(&b);
    }
    // the pictures are made off the main thread, a few workers at once, and
    // put in as they come (the window stays smooth meanwhile)
    let (tx, rx) = std::sync::mpsc::channel::<(usize, u32, u32, Vec<u8>)>();
    let paths: Vec<PathBuf> = pending.iter().map(|p| p.1.clone()).collect();
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 6);
    for w in 0..workers {
        let (tx, paths) = (tx.clone(), paths.clone());
        std::thread::spawn(move || {
            for (k, path) in paths.iter().enumerate().skip(w).step_by(workers) {
                if let Some((pw, ph, px)) = crate::pages::thumb_pixels(path, 52) {
                    if tx.send((k, pw, ph, px)).is_err() {
                        return;
                    }
                }
            }
        });
    }
    drop(tx);
    glib::timeout_add_local(std::time::Duration::from_millis(30), move || {
        for _ in 0..48 {
            match rx.try_recv() {
                Ok((k, w, h, px)) => {
                    if let Some((pic, _)) = pending.get(k) {
                        pic.set_paintable(Some(&crate::pages::texture(w, h, px)));
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return glib::ControlFlow::Break,
            }
        }
        glib::ControlFlow::Continue
    });
    let (s, ds) = (search.clone(), dirs.clone());
    grid.set_filter_func(move |child| {
        let q = s.text().trim().to_lowercase();
        q.is_empty() || ds.get(child.index() as usize).is_some_and(|d| d.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().contains(&q)))
    });
    let g = grid.clone();
    search.connect_search_changed(move |_| g.invalidate_filter());
    let side = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_start(8).margin_end(10).margin_top(8).margin_bottom(8).build();
    side.append(&search);
    side.append(&gtk::ScrolledWindow::builder().child(&grid).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build());
    side.upcast()
}

/// A character's outfits, in the little window that opens from it.
fn outfits(r: &Rc<Roster>, dir: &Path, pop: &gtk::Popover) -> gtk::Widget {
    let flow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(3).min_children_per_line(1).row_spacing(6).column_spacing(6).build();
    let in_team: Vec<String> = r.ui.data.borrow().team.iter().map(|m| m.path.clone()).collect();
    for o in team::outfits(dir) {
        let path = o.calm.to_string_lossy().into_owned();
        let already = in_team.contains(&path);
        // calm on the left, happy on the right
        let pics = gtk::Box::builder().spacing(2).halign(gtk::Align::Center).build();
        pics.append(&thumb(&r.ui, &path, 72));
        if let Some(h) = &o.happy {
            pics.append(&thumb(&r.ui, &h.to_string_lossy(), 72));
        }
        let inner = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
        inner.append(&pics);
        let label = gtk::Label::builder().label(if already { format!("✓ {}", o.name) } else { o.name.clone() }).wrap(true).max_width_chars(14).justify(gtk::Justification::Center).build();
        label.add_css_class("caption");
        inner.append(&label);
        let b = gtk::Button::builder().child(&inner).sensitive(!already).tooltip_text(if already { "Already in the line-up" } else { "Add to the line-up" }).build();
        b.add_css_class("flat");
        let (rr, p) = (r.clone(), pop.clone());
        b.connect_clicked(move |_| {
            p.popdown();
            let p2 = path.clone();
            edit(&rr, move |d| {
                if !d.team.iter().any(|m| m.path == p2) {
                    d.team.push(team::Member { path: p2, on: true, crew: String::new() });
                }
            });
        });
        flow.append(&b);
    }
    let title = gtk::Label::builder().label(dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).xalign(0.0).margin_bottom(4).build();
    title.add_css_class("heading");
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    b.append(&title);
    b.append(&gtk::ScrolledWindow::builder().child(&flow).propagate_natural_height(true).max_content_height(460).min_content_width(480).hscrollbar_policy(gtk::PolicyType::Never).build());
    b.upcast()
}

fn save_button(r: &Rc<Roster>) -> gtk::MenuButton {
    let entry = gtk::Entry::builder().placeholder_text("Name, e.g. Evening crew").width_chars(22).build();
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    let row = gtk::Box::builder().spacing(6).build();
    row.append(&entry);
    row.append(&save);
    let pop = gtk::Popover::builder().child(&row).build();
    let button = gtk::MenuButton::builder().label("Save as line-up…").popover(&pop).tooltip_text("Save these characters under a name").build();
    let go = {
        let (rr, e, p) = (r.clone(), entry.clone(), pop.clone());
        Rc::new(move || {
            let name = e.text().trim().to_string();
            if name.is_empty() || rr.ui.data.borrow().team.is_empty() {
                return;
            }
            let existed = rr.ui.data.borrow().groups.contains_key(&name);
            {
                let mut d = rr.ui.data.borrow_mut();
                let team = d.team.clone();
                d.groups.insert(name.clone(), team);
                d.save();
            }
            e.set_text("");
            p.popdown();
            fill_lineups(&rr);
            rr.ui.say(&if existed { format!("Updated \"{name}\"") } else { format!("Saved \"{name}\"") });
        })
    };
    let g = go.clone();
    save.connect_clicked(move |_| g());
    entry.connect_activate(move |_| go());
    button
}

fn settings_row(ui: &Rc<Ui>) -> gtk::Widget {
    let doc = config::read();
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.2, 1.5, 0.05);
    scale.set_hexpand(true);
    scale.set_value(config::number(&doc, "character", "scale", 0.45));
    let (u, wait) = (ui.clone(), crate::pages::Debounce::default());
    scale.connect_value_changed(move |s| {
        let (u, v) = (u.clone(), (s.value() * 100.0).round() / 100.0);
        wait.call(400, move || {
            config::set("character", "scale", v);
            u.restart_soon();
        });
    });
    let toggle = |key: &'static str, default: bool| {
        let sw = gtk::Switch::builder().valign(gtk::Align::Center).active(config::boolean(&doc, "character", key, default)).build();
        let u = ui.clone();
        sw.connect_active_notify(move |w| {
            config::set("character", key, w.is_active());
            // applied as it runs: nobody gets reloaded
            u.reload_soon();
        });
        sw
    };
    let row = gtk::Box::builder().spacing(8).build();
    row.append(&gtk::Label::new(Some("Size")));
    row.append(&scale);
    row.append(&gtk::Label::new(Some("Follow mouse")));
    row.append(&toggle("look_at_mouse", true));
    let race = gtk::Button::with_label("Race");
    race.connect_clicked(|_| daemon::race());
    row.append(&race);
    row.upcast()
}

pub fn page(ui: &Rc<Ui>) -> gtk::Widget {
    let r = Rc::new(Roster {
        ui: ui.clone(),
        cards: gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(12)
            .min_children_per_line(3)
            .row_spacing(10)
            .column_spacing(10)
            .valign(gtk::Align::Start)
            .homogeneous(true)
            .build(),
        card_rows: std::cell::RefCell::new(Vec::new()),
        summary: gtk::Label::builder().xalign(0.0).build(),
        apply: gtk::Button::builder().label("Apply").tooltip_text("Show this line-up on screen").build(),
        pending: small("Not applied yet"),
        lineups: gtk::Box::new(gtk::Orientation::Vertical, 0),
    });
    r.summary.add_css_class("heading");
    r.apply.add_css_class("suggested-action");
    let rr = r.clone();
    r.apply.connect_clicked(move |_| apply(&rr));

    // toolbar
    let list = gtk::ScrolledWindow::builder().child(&r.lineups).propagate_natural_height(true).max_content_height(460).min_content_width(420).build();
    let saved_pop = gtk::Popover::builder().child(&list).build();
    let saved = gtk::MenuButton::builder().label("Line-ups").popover(&saved_pop).tooltip_text("Saved line-ups").build();
    let bar = gtk::Box::builder().spacing(8).margin_start(10).margin_end(10).margin_top(8).margin_bottom(6).build();
    bar.append(&r.summary);
    bar.append(&r.pending);
    bar.append(&gtk::Box::builder().hexpand(true).build());
    bar.append(&saved);
    bar.append(&save_button(&r));
    bar.append(&r.apply);
    let add = gtk::ToggleButton::builder().tooltip_text("Show or hide the character picker").build();
    let add_label = gtk::Label::new(Some("‹  Add characters"));
    add.set_child(Some(&add_label));
    add.add_css_class("suggested-action");
    bar.append(&add);

    // left: the line-up; right: characters to add
    let left = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_start(10).margin_end(8).margin_top(4).margin_bottom(8).build();
    left.append(&gtk::ScrolledWindow::builder().child(&r.cards).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build());
    left.append(&small("Switch: on screen · Right-click: status, friendships, remove"));
    let bottom = settings_row(ui);
    bottom.add_css_class("bottom-bar");
    left.append(&bottom);
    left.set_hexpand(true);
    let pick_holder = gtk::Box::builder().orientation(gtk::Orientation::Vertical).vexpand(true).build();
    pick_holder.add_css_class("picker");
    // the picker slides in over the line-up from the right, and out again
    let paned = adw::OverlaySplitView::builder()
        .content(&left)
        .sidebar(&pick_holder)
        .sidebar_position(gtk::PackType::End)
        .collapsed(true)
        .show_sidebar(false)
        .min_sidebar_width(380.0)
        .max_sidebar_width(560.0)
        .sidebar_width_fraction(0.45)
        .vexpand(true)
        .build();
    let (pv, lbl) = (paned.clone(), add_label.clone());
    add.connect_toggled(move |b| {
        let open = b.is_active();
        pv.set_show_sidebar(open);
        lbl.set_label(if open { "Done  ›" } else { "‹  Add characters" });
    });
    let (tb, holder, rr, loaded) = (add.clone(), pick_holder.clone(), r.clone(), std::cell::Cell::new(false));
    paned.connect_show_sidebar_notify(move |p| {
        if p.shows_sidebar() && !loaded.get() {
            loaded.set(true);
            holder.append(&picker(&rr));
        }
        if tb.is_active() != p.shows_sidebar() {
            tb.set_active(p.shows_sidebar());
        }
    });

    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.append(&bar);
    page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    page.append(&paned);
    refresh(&r);
    page.upcast()
}

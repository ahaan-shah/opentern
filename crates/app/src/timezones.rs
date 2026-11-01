//! The chart's timezone: Exchange, UTC, this computer's, or a city (as
//! TradingView's list). Picked from the clock in the range bar or in Chart
//! settings → Symbol.

use egui::Ui;
use jiff::tz::TimeZone;

use crate::chart::style::TimeZoneChoice;
use crate::theme::Theme;
use crate::widgets as w;

/// Cities offered, by IANA name. Shown sorted by their offset today.
const CITIES: &[(&str, &str)] = &[
    ("Pacific/Honolulu", "Honolulu"),
    ("America/Anchorage", "Anchorage"),
    ("America/Los_Angeles", "Los Angeles"),
    ("America/Vancouver", "Vancouver"),
    ("America/Phoenix", "Phoenix"),
    ("America/Denver", "Denver"),
    ("America/Mexico_City", "Mexico City"),
    ("America/Chicago", "Chicago"),
    ("America/Bogota", "Bogotá"),
    ("America/Lima", "Lima"),
    ("America/New_York", "New York"),
    ("America/Toronto", "Toronto"),
    ("America/Caracas", "Caracas"),
    ("America/Santiago", "Santiago"),
    ("America/Halifax", "Halifax"),
    ("America/Sao_Paulo", "São Paulo"),
    ("America/Argentina/Buenos_Aires", "Buenos Aires"),
    ("Atlantic/Reykjavik", "Reykjavík"),
    ("Europe/London", "London"),
    ("Europe/Dublin", "Dublin"),
    ("Europe/Lisbon", "Lisbon"),
    ("Africa/Casablanca", "Casablanca"),
    ("Africa/Lagos", "Lagos"),
    ("Europe/Paris", "Paris"),
    ("Europe/Berlin", "Berlin"),
    ("Europe/Madrid", "Madrid"),
    ("Europe/Rome", "Rome"),
    ("Europe/Amsterdam", "Amsterdam"),
    ("Europe/Brussels", "Brussels"),
    ("Europe/Zurich", "Zurich"),
    ("Europe/Vienna", "Vienna"),
    ("Europe/Stockholm", "Stockholm"),
    ("Europe/Oslo", "Oslo"),
    ("Europe/Copenhagen", "Copenhagen"),
    ("Europe/Warsaw", "Warsaw"),
    ("Europe/Belgrade", "Belgrade"),
    ("Africa/Johannesburg", "Johannesburg"),
    ("Africa/Cairo", "Cairo"),
    ("Europe/Athens", "Athens"),
    ("Europe/Helsinki", "Helsinki"),
    ("Europe/Riga", "Riga"),
    ("Europe/Tallinn", "Tallinn"),
    ("Europe/Vilnius", "Vilnius"),
    ("Asia/Jerusalem", "Jerusalem"),
    ("Europe/Istanbul", "Istanbul"),
    ("Europe/Moscow", "Moscow"),
    ("Africa/Nairobi", "Nairobi"),
    ("Asia/Riyadh", "Riyadh"),
    ("Asia/Kuwait", "Kuwait"),
    ("Asia/Bahrain", "Bahrain"),
    ("Asia/Qatar", "Qatar"),
    ("Asia/Tehran", "Tehran"),
    ("Asia/Dubai", "Dubai"),
    ("Asia/Muscat", "Muscat"),
    ("Asia/Karachi", "Karachi"),
    ("Asia/Tashkent", "Tashkent"),
    ("Asia/Kolkata", "Kolkata"),
    ("Asia/Colombo", "Colombo"),
    ("Asia/Kathmandu", "Kathmandu"),
    ("Asia/Dhaka", "Dhaka"),
    ("Asia/Almaty", "Almaty"),
    ("Asia/Yangon", "Yangon"),
    ("Asia/Bangkok", "Bangkok"),
    ("Asia/Ho_Chi_Minh", "Ho Chi Minh City"),
    ("Asia/Jakarta", "Jakarta"),
    ("Asia/Shanghai", "Shanghai"),
    ("Asia/Hong_Kong", "Hong Kong"),
    ("Asia/Taipei", "Taipei"),
    ("Asia/Singapore", "Singapore"),
    ("Asia/Kuala_Lumpur", "Kuala Lumpur"),
    ("Asia/Manila", "Manila"),
    ("Australia/Perth", "Perth"),
    ("Asia/Seoul", "Seoul"),
    ("Asia/Tokyo", "Tokyo"),
    ("Australia/Adelaide", "Adelaide"),
    ("Australia/Brisbane", "Brisbane"),
    ("Australia/Sydney", "Sydney"),
    ("Pacific/Norfolk", "Norfolk Island"),
    ("Pacific/Auckland", "New Zealand"),
    ("Pacific/Chatham", "Chatham Islands"),
    ("Pacific/Fakaofo", "Tokelau"),
];

/// `UTC+5:30`, `UTC−4`, `UTC` for the zone's offset right now.
pub fn offset_label(tz: &TimeZone) -> String {
    let secs = tz.to_offset(jiff::Timestamp::now()).seconds();
    if secs == 0 {
        return "UTC".into();
    }
    let sign = if secs < 0 { '−' } else { '+' };
    let (h, m) = (secs.abs() / 3600, secs.abs() % 3600 / 60);
    if m == 0 {
        format!("UTC{sign}{h}")
    } else {
        format!("UTC{sign}{h}:{m:02}")
    }
}

/// The city name for an IANA zone, or the zone itself.
pub fn city(name: &str) -> &str {
    CITIES.iter().find(|(z, _)| *z == name).map(|(_, c)| *c).unwrap_or(name)
}

/// What the choice is called in menus and buttons.
pub fn choice_name(c: &TimeZoneChoice) -> String {
    match c {
        TimeZoneChoice::Exchange => "Exchange".into(),
        TimeZoneChoice::Utc => "UTC".into(),
        TimeZoneChoice::Local => "Local".into(),
        TimeZoneChoice::Zone(z) => city(z).to_string(),
    }
}

/// The picker: a search field, then Exchange, UTC and Local, then every
/// city by offset. `exchange` is the symbol's own zone (for its offset).
/// Returns true when the choice changed.
pub fn picker(ui: &mut Ui, t: &Theme, id: egui::Id, current: &mut TimeZoneChoice, exchange: &TimeZone) -> bool {
    let mut changed = false;
    let mut query: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let r = w::search_field(ui, t, &mut query, "Search", ui.available_width());
    if !r.has_focus() && query.is_empty() && ui.memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
    ui.add_space(4.0);
    let q = query.trim().to_lowercase();
    let hit = |name: &str, zone: &str, offset: &str| {
        q.is_empty()
            || name.to_lowercase().contains(&q)
            || zone.to_lowercase().contains(&q)
            || offset.to_lowercase().contains(&q)
    };
    let mut pick = |ui: &mut Ui, choice: TimeZoneChoice, label: &str, offset: &str| {
        let on = *current == choice;
        if w::menu_item(ui, t, None, label, offset, on).clicked() {
            if !on {
                *current = choice;
                changed = true;
            }
            ui.close();
        }
    };
    egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        let mut any = false;
        let fixed = [
            (TimeZoneChoice::Exchange, "Exchange", offset_label(exchange)),
            (TimeZoneChoice::Utc, "UTC", "UTC".to_string()),
            (TimeZoneChoice::Local, "Local", offset_label(&TimeZone::system())),
        ];
        for (choice, label, offset) in fixed {
            if hit(label, "", &offset) {
                pick(ui, choice, label, &offset);
                any = true;
            }
        }
        let mut cities: Vec<(i32, &str, &str, String)> = CITIES
            .iter()
            .filter_map(|(zone, name)| {
                let tz = TimeZone::get(zone).ok()?;
                let secs = tz.to_offset(jiff::Timestamp::now()).seconds();
                Some((secs, *zone, *name, offset_label(&tz)))
            })
            .filter(|(_, zone, name, offset)| hit(name, zone, offset))
            .collect();
        cities.sort_by(|a, b| a.0.cmp(&b.0).then(a.2.cmp(b.2)));
        if any && !cities.is_empty() {
            w::menu_rule(ui, t);
        }
        for (_, zone, name, offset) in &cities {
            pick(ui, TimeZoneChoice::Zone(zone.to_string()), name, offset);
        }
        if !any && cities.is_empty() {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new("No matches").color(t.text3)));
            ui.add_space(8.0);
        }
    });
    if changed {
        query.clear();
    }
    ui.data_mut(|d| d.insert_temp(id, query));
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_city_is_a_known_zone() {
        for (zone, _) in CITIES {
            assert!(TimeZone::get(zone).is_ok(), "{zone}");
        }
    }

    #[test]
    fn offsets_read_like_tradingview() {
        assert_eq!(offset_label(&TimeZone::UTC), "UTC");
        assert_eq!(offset_label(&TimeZone::get("Asia/Kolkata").unwrap()), "UTC+5:30");
        assert_eq!(offset_label(&TimeZone::get("Asia/Kathmandu").unwrap()), "UTC+5:45");
    }

    #[test]
    fn old_choices_still_load() {
        let c: TimeZoneChoice = serde_json::from_str("\"Exchange\"").unwrap();
        assert_eq!(c, TimeZoneChoice::Exchange);
        let z = TimeZoneChoice::Zone("Asia/Tokyo".into());
        let back: TimeZoneChoice = serde_json::from_str(&serde_json::to_string(&z).unwrap()).unwrap();
        assert_eq!(back, z);
    }
}

//! The game's own titles for its maps. The log only names the scenario file.

use crate::display_link::short_map;
use crate::i18n::Lang;

/// `scenario <tab> English <tab> German`, written by `examples/update_maps.rs`.
const TABLE: &str = include_str!("maps.tsv");

fn rows() -> impl Iterator<Item = (&'static str, &'static str, &'static str)> {
    TABLE.lines().filter(|line| !line.starts_with('#')).filter_map(|line| {
        let mut fields = line.split('\t');
        Some((fields.next()?, fields.next()?, fields.next().unwrap_or("")))
    })
}

/// `crossing_in_the_woods_4p` -> `Crossing In The Woods`, for maps the table does not know.
fn from_file_name(map: &str) -> String {
    short_map(map)
        .split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            let first = chars.next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            first + chars.as_str()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The title the game shows for a scenario, without its player count:
/// `rural_town_4p` -> `Pachino Farmlands`, or `Pachino-Ackerland` in German.
pub fn title(map: &str, lang: Lang) -> String {
    match rows().find(|(id, ..)| *id == map) {
        Some((_, _, german)) if lang == Lang::De && !german.is_empty() => german.to_string(),
        Some((_, english, _)) => english.to_string(),
        None => from_file_name(map),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_the_games_titles() {
        assert_eq!(title("rural_town_4p", Lang::En), "Pachino Farmlands");
        assert_eq!(title("rural_town_4p", Lang::De), "Pachino-Ackerland");
        assert_eq!(title("crossing_in_the_woods_4p", Lang::En), "Crossing in the Woods");
        // Not every scenario is named after its size.
        assert_eq!(title("eindhoven", Lang::En), "Operation Eindhoven");
        assert_eq!(title("desert_airfield_6p_mkii", Lang::De), "Landeplatz Gazala");
    }

    #[test]
    fn uses_english_where_the_game_has_no_german_title() {
        assert_eq!(title("ancona_railyard_2p", Lang::De), "Ancona Railyard");
    }

    #[test]
    fn falls_back_to_the_file_name() {
        assert_eq!(title("some_new_map_6p", Lang::De), "Some New Map");
        assert_eq!(title("twin_beach", Lang::En), "Twin Beach");
    }

    /// Guards a refresh of the table: what the display cannot show or fit must not get in.
    #[test]
    fn every_title_suits_the_display() {
        assert!(rows().count() > 60);
        for (id, english, german) in rows() {
            assert!(!english.is_empty(), "{id} has no title");
            for title in [english, german] {
                assert!(title.chars().count() <= display_protocol::frame::MAX_MAP, "{title} is too long");
                assert!(title.chars().all(|c| (c as u32) <= 0xFF && !c.is_control()), "{title} is not Latin-1");
            }
        }
    }
}

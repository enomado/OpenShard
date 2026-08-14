//! The client-owned NPC vendor window.
//!
//! The shop packets carry rows, not a `0xB0` layout, so this is deliberately a
//! small native gump: it gives the otherwise wire-only catalogue prices,
//! quantities and controls while keeping all payment decisions on the shard.

use openshard_protocol::containers::ContainedItem;
use openshard_protocol::serial::Serial;
use openshard_protocol::speech::Font;
use openshard_protocol::vendor::{BuyLine, SellLine};
use openshard_protocol::wire::Hue;

use crate::gump::{self, GumpArt, GumpAtlas, GumpPixel, Picture};
use crate::text::GumpLabel;

pub const WIDTH: i32 = 300;
pub const ROW_HEIGHT: i32 = 17;
const HEADER: i32 = 40;
const FOOTER: i32 = 34;
pub const VISIBLE_ROWS: usize = 7;
const BACKGROUND: u16 = 0x0A3C;
const TITLE_FONT: Font = Font(6);
const ROW_FONT: Font = Font(9);
const TITLE_HUE: Hue = Hue(0x0386);
const ROW_HUE: Hue = Hue(0x0481);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Row(usize),
    Confirm,
    Close,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Line {
    pub at: GumpPixel,
    pub text: String,
    pub font: Font,
    pub hue: Hue,
    pub clip: Option<(i32, i32)>,
}

impl Line {
    pub fn label(&self) -> GumpLabel<'_> {
        GumpLabel {
            at: self.at,
            text: &self.text,
            font: self.font,
            hue: self.hue,
            clip: self.clip,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Window {
    pub vendor: Serial,
    pub sell: bool,
    pub at: GumpPixel,
    pub scroll: usize,
    pub rows: usize,
    pub pictures: Vec<Picture>,
    pub lines: Vec<Line>,
}

impl Window {
    pub fn height(&self) -> i32 {
        HEADER + VISIBLE_ROWS as i32 * ROW_HEIGHT + FOOTER
    }

    pub fn hit(&self, cursor: GumpPixel) -> Option<Hit> {
        let x = cursor.x - self.at.x;
        let y = cursor.y - self.at.y;
        if !(0..WIDTH).contains(&x) || !(0..self.height()).contains(&y) {
            return None;
        }
        if (HEADER..HEADER + self.rows as i32 * ROW_HEIGHT).contains(&y) {
            return Some(Hit::Row(self.scroll + ((y - HEADER) / ROW_HEIGHT) as usize));
        }
        if y >= HEADER + VISIBLE_ROWS as i32 * ROW_HEIGHT {
            return Some(if x < WIDTH / 2 { Hit::Confirm } else { Hit::Close });
        }
        Some(Hit::Close)
    }
}

pub fn art_of() -> impl Iterator<Item = GumpArt> {
    (0..9).map(|offset| GumpArt::Gump(openshard_protocol::wire::Graphic(BACKGROUND + offset)))
}

struct Row {
    number: usize,
    name: String,
    price: u32,
    quantity: String,
    graphic: Option<openshard_protocol::wire::Graphic>,
    hue: Hue,
}

pub fn buy(
    vendor: Serial,
    lines: &[BuyLine],
    items: &[ContainedItem],
    amounts: &[u16],
    scroll: usize,
    at: GumpPixel,
    atlas: &GumpAtlas,
) -> Window {
    window(
        vendor,
        false,
        lines.iter().enumerate().map(|(i, line)| Row {
            number: i + 1,
            name: line.name.clone(),
            price: u32::from(line.price),
            quantity: format!("x{}", amounts.get(i).copied().unwrap_or(0)),
            graphic: items.get(i).map(|item| item.graphic),
            hue: items.get(i).map_or(Hue::NONE, |item| item.hue),
        }),
        scroll,
        at,
        atlas,
    )
}

pub fn sell(
    vendor: Serial,
    lines: &[SellLine],
    amounts: &[u16],
    scroll: usize,
    at: GumpPixel,
    atlas: &GumpAtlas,
) -> Window {
    window(
        vendor,
        true,
        lines.iter().enumerate().map(|(i, line)| Row {
            number: i + 1,
            name: line.name.clone(),
            price: u32::from(line.price),
            quantity: format!("{}/{}", amounts.get(i).copied().unwrap_or(0), line.amount),
            graphic: Some(line.graphic),
            hue: line.hue,
        }),
        scroll,
        at,
        atlas,
    )
}

fn window(
    vendor: Serial,
    sell: bool,
    rows: impl Iterator<Item = Row>,
    scroll: usize,
    at: GumpPixel,
    atlas: &GumpAtlas,
) -> Window {
    let rows: Vec<Row> = rows.collect();
    let scroll = scroll.min(rows.len().saturating_sub(VISIBLE_ROWS));
    let mut lines = vec![Line {
        at: at.offset(GumpPixel::new(8, 5)),
        text: if sell { "SELL TO VENDOR" } else { "BUY FROM VENDOR" }.to_owned(),
        font: TITLE_FONT,
        hue: TITLE_HUE,
        clip: None,
    }];
    for (x, text) in [(10, "#"), (30, "ITEM"), (190, "PRICE"), (255, "QTY")] {
        lines.push(Line {
            at: at.offset(GumpPixel::new(x, 26)),
            text: text.to_owned(),
            font: ROW_FONT,
            hue: TITLE_HUE,
            clip: None,
        });
    }
    lines.extend(
        rows.iter()
            .skip(scroll)
            .take(VISIBLE_ROWS)
            .enumerate()
            .flat_map(|(i, row)| {
                let y = HEADER + i as i32 * ROW_HEIGHT;
                [
                    Line {
                        at: at.offset(GumpPixel::new(10, y)),
                        text: format!("{:02}", row.number),
                        font: ROW_FONT,
                        hue: ROW_HUE,
                        clip: None,
                    },
                    Line {
                        at: at.offset(GumpPixel::new(62, y)),
                        text: row.name.clone(),
                        font: ROW_FONT,
                        hue: ROW_HUE,
                        clip: Some((122, ROW_HEIGHT)),
                    },
                    Line {
                        at: at.offset(GumpPixel::new(192, y)),
                        text: format!("{} gp", row.price),
                        font: ROW_FONT,
                        hue: ROW_HUE,
                        clip: Some((58, ROW_HEIGHT)),
                    },
                    Line {
                        at: at.offset(GumpPixel::new(255, y)),
                        text: row.quantity.clone(),
                        font: ROW_FONT,
                        hue: ROW_HUE,
                        clip: Some((35, ROW_HEIGHT)),
                    },
                ]
            }),
    );
    let bottom = HEADER + VISIBLE_ROWS as i32 * ROW_HEIGHT + 7;
    lines.push(Line {
        at: at.offset(GumpPixel::new(8, bottom)),
        text: if sell { "SELL" } else { "BUY" }.to_owned(),
        font: TITLE_FONT,
        hue: TITLE_HUE,
        clip: None,
    });
    lines.push(Line {
        at: at.offset(GumpPixel::new(WIDTH / 2 + 8, bottom)),
        text: "CANCEL".to_owned(),
        font: TITLE_FONT,
        hue: TITLE_HUE,
        clip: None,
    });
    Window {
        vendor,
        sell,
        at,
        scroll,
        rows: rows.len().saturating_sub(scroll).min(VISIBLE_ROWS),
        pictures: {
            let mut pictures = gump::resize(
                atlas,
                openshard_protocol::wire::Graphic(BACKGROUND),
                at,
                WIDTH,
                HEADER + VISIBLE_ROWS as i32 * ROW_HEIGHT + FOOTER,
            );
            pictures.extend(
                rows.iter()
                    .skip(scroll)
                    .take(VISIBLE_ROWS)
                    .enumerate()
                    .filter_map(|(i, row)| {
                        row.graphic.map(|graphic| {
                            let icon_at = at.offset(GumpPixel::new(30, HEADER + i as i32 * ROW_HEIGHT));
                            Picture::plain(GumpArt::Item(graphic), icon_at)
                                .hued(row.hue)
                                .inside(gump::Scissor {
                                    at: icon_at,
                                    width: 18,
                                    height: ROW_HEIGHT,
                                })
                        })
                    }),
            );
            pictures
        },
        lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rows_and_footer_have_disjoint_hits() {
        let window = buy(
            Serial::new(42).unwrap(),
            &[BuyLine {
                price: 5,
                name: "apple".into(),
            }],
            &[],
            &[0],
            0,
            GumpPixel::new(10, 20),
            &GumpAtlas::empty(),
        );
        assert_eq!(window.hit(GumpPixel::new(12, 65)), Some(Hit::Row(0)));
        assert_eq!(window.hit(GumpPixel::new(12, 192)), Some(Hit::Confirm));
        assert_eq!(window.hit(GumpPixel::new(200, 192)), Some(Hit::Close));
    }

    #[test]
    fn a_long_catalogue_has_a_fixed_viewport_and_offsets_its_row_hits() {
        let lines: Vec<BuyLine> = (0..13)
            .map(|price| BuyLine {
                price,
                name: format!("item {price}"),
            })
            .collect();
        let window = buy(
            Serial::new(42).unwrap(),
            &lines,
            &[],
            &vec![0; lines.len()],
            1,
            GumpPixel::new(10, 20),
            &GumpAtlas::empty(),
        );

        assert_eq!(window.rows, VISIBLE_ROWS);
        assert_eq!(window.lines.len(), 1 + 4 + VISIBLE_ROWS * 4 + 2);
        assert!(window.lines.iter().any(|line| line.text == "02"));
        assert_eq!(window.hit(GumpPixel::new(12, 61)), Some(Hit::Row(1)));
    }
}

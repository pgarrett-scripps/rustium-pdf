//! Glyph outlines in font units, shared by every font program parser.

use crate::geom::{Point, Rect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCmd {
    MoveTo(Point),
    LineTo(Point),
    CurveTo(Point, Point, Point),
    Close,
}

/// One glyph's outline and horizontal advance, in the font program's units.
#[derive(Debug, Clone, Default)]
pub struct Outline {
    pub cmds: Vec<PathCmd>,
    pub advance: f32,
}

impl Outline {
    /// The ink bounding box: control points included, which slightly over-covers curves but
    /// never under-covers, and matches what glyph-box consumers need.
    pub fn bbox(&self) -> Option<Rect> {
        let mut r: Option<Rect> = None;
        let mut push = |p: Point| {
            r = Some(match r {
                None => Rect {
                    x0: p.x,
                    y0: p.y,
                    x1: p.x,
                    y1: p.y,
                },
                Some(b) => Rect {
                    x0: b.x0.min(p.x),
                    y0: b.y0.min(p.y),
                    x1: b.x1.max(p.x),
                    y1: b.y1.max(p.y),
                },
            });
        };
        for cmd in &self.cmds {
            match *cmd {
                PathCmd::MoveTo(p) | PathCmd::LineTo(p) => push(p),
                PathCmd::CurveTo(c1, c2, p) => {
                    push(c1);
                    push(c2);
                    push(p);
                }
                PathCmd::Close => {}
            }
        }
        r
    }
}

/// Accumulates path commands during charstring interpretation.
#[derive(Default)]
pub struct OutlineBuilder {
    pub cmds: Vec<PathCmd>,
    pub x: f32,
    pub y: f32,
    open: bool,
}

impl OutlineBuilder {
    pub fn move_to(&mut self, x: f32, y: f32) {
        if self.open {
            self.cmds.push(PathCmd::Close);
        }
        self.x = x;
        self.y = y;
        self.cmds.push(PathCmd::MoveTo(Point::new(x, y)));
        self.open = true;
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        self.x = x;
        self.y = y;
        self.cmds.push(PathCmd::LineTo(Point::new(x, y)));
    }

    pub fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.x = x;
        self.y = y;
        self.cmds.push(PathCmd::CurveTo(
            Point::new(x1, y1),
            Point::new(x2, y2),
            Point::new(x, y),
        ));
    }

    /// Closes the current contour. Idempotent: charstring interpreters and `ttf-parser` both
    /// emit an explicit close that `move_to` may already have inserted.
    pub fn close(&mut self) {
        if self.open {
            self.cmds.push(PathCmd::Close);
            self.open = false;
        }
    }

    pub fn finish(mut self) -> Vec<PathCmd> {
        if self.open {
            self.cmds.push(PathCmd::Close);
        }
        self.cmds
    }
}

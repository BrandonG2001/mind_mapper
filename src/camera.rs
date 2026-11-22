//! Pan/zoom transform between world (map) coordinates and screen pixels.

use eframe::egui::{Pos2, Vec2};

use crate::model::Point;

/// Smallest allowed zoom factor.
pub const MIN_ZOOM: f32 = 0.1;
/// Largest allowed zoom factor.
pub const MAX_ZOOM: f32 = 5.0;

/// Maps world coordinates to screen coordinates: `screen = offset + world * zoom`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    offset: Vec2,
    zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            offset: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera {
    /// Current zoom factor (1.0 = 100%).
    #[must_use]
    pub const fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Convert a world point to screen pixels.
    #[must_use]
    pub fn world_to_screen(&self, world: Point) -> Pos2 {
        Pos2::new(world.x, world.y) * self.zoom + self.offset
    }

    /// Convert screen pixels to a world point.
    #[must_use]
    pub fn screen_to_world(&self, screen: Pos2) -> Point {
        let world = (screen - self.offset) / self.zoom;
        Point::new(world.x, world.y)
    }

    /// Shift the view by a screen-space delta (e.g. a mouse drag).
    pub fn pan(&mut self, screen_delta: Vec2) {
        self.offset += screen_delta;
    }

    /// Multiply the zoom by `factor` while keeping the world point under `anchor` fixed,
    /// so the map zooms towards the mouse cursor.
    ///
    /// # Arguments
    ///
    /// * `anchor` - Screen position that stays put (usually the cursor)
    /// * `factor` - Multiplier; the result is clamped to `MIN_ZOOM..=MAX_ZOOM`
    pub fn zoom_at(&mut self, anchor: Pos2, factor: f32) {
        let world = self.screen_to_world(anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.offset = anchor - Pos2::new(world.x, world.y) * self.zoom;
    }

    /// Reset to 100% zoom with `world` drawn at screen position `screen`.
    pub fn center_on(&mut self, world: Point, screen: Pos2) {
        self.zoom = 1.0;
        self.offset = screen - Pos2::new(world.x, world.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(a: Pos2, b: Pos2) {
        assert!((a - b).length() < 1e-3, "{a:?} != {b:?}");
    }

    #[test]
    fn transforms_are_inverse() {
        let mut camera = Camera::default();
        camera.pan(Vec2::new(30.0, -12.0));
        camera.zoom_at(Pos2::new(5.0, 5.0), 2.5);
        let world = Point::new(17.0, -4.0);

        let back = camera.screen_to_world(camera.world_to_screen(world));

        assert_close(Pos2::new(back.x, back.y), Pos2::new(world.x, world.y));
    }

    #[test]
    fn zoom_keeps_anchor_fixed() {
        let mut camera = Camera::default();
        let anchor = Pos2::new(100.0, 50.0);
        let before = camera.screen_to_world(anchor);

        camera.zoom_at(anchor, 3.0);

        assert_close(camera.world_to_screen(before), anchor);
        assert_eq!(camera.zoom(), 3.0);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut camera = Camera::default();

        camera.zoom_at(Pos2::ZERO, 1000.0);
        assert_eq!(camera.zoom(), MAX_ZOOM);
        camera.zoom_at(Pos2::ZERO, 0.0);
        assert_eq!(camera.zoom(), MIN_ZOOM);
    }

    #[test]
    fn center_on_places_world_point() {
        let mut camera = Camera::default();
        camera.zoom_at(Pos2::ZERO, 2.0);

        camera.center_on(Point::new(10.0, 20.0), Pos2::new(400.0, 300.0));

        assert_eq!(camera.zoom(), 1.0);
        assert_close(
            camera.world_to_screen(Point::new(10.0, 20.0)),
            Pos2::new(400.0, 300.0),
        );
    }
}

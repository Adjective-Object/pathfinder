// pathfinder/geometry/src/stroke.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Utilities for converting path strokes to fills.

use crate::basic::line_segment::LineSegmentF32;
use crate::basic::point::Point2DF32;
use crate::basic::rect::RectF32;
use crate::outline::{Contour, Outline};
use crate::segment::Segment;
use std::mem;

const TOLERANCE: f32 = 0.01;

pub struct OutlineStrokeToFill {
    pub outline: Outline,
    pub style: StrokeStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeStyle {
    pub line_width: f32,
    pub line_cap: LineCap,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineCap {
    Butt,
    Square,
}

impl OutlineStrokeToFill {
    #[inline]
    pub fn new(outline: Outline, style: StrokeStyle) -> OutlineStrokeToFill {
        OutlineStrokeToFill { outline, style }
    }

    pub fn offset(&mut self) {
        let mut new_contours = vec![];
        for input in mem::replace(&mut self.outline.contours, vec![]) {
            let closed = input.closed;
            let mut stroker = ContourStrokeToFill::new(input,
                                                       Contour::new(),
                                                       self.style.line_width * 0.5);

            stroker.offset_forward();
            if closed {
                stroker.output.closed = true;
                new_contours.push(stroker.output);
                stroker = ContourStrokeToFill::new(stroker.input,
                                                   Contour::new(),
                                                   self.style.line_width * 0.5);
            } else {
                self.add_cap(&mut stroker.output);
            }

            stroker.offset_backward();
            if !closed {
                self.add_cap(&mut stroker.output);
            }

            stroker.output.closed = true;
            new_contours.push(stroker.output);
        }

        let mut new_bounds = None;
        new_contours.iter().for_each(|contour| contour.update_bounds(&mut new_bounds));

        self.outline.contours = new_contours;
        self.outline.bounds = new_bounds.unwrap_or_else(|| RectF32::default());
    }

    pub fn add_cap(&mut self, contour: &mut Contour) {
        if self.style.line_cap == LineCap::Butt || contour.len() < 2 {
            return
        }

        let width = self.style.line_width;
        let (p0, p1) = (contour.position_of_last(2), contour.position_of_last(1));
        let gradient = (p1 - p0).normalize();
        let offset = gradient.scale(width * 0.5);

        let p2 = p1 + offset;
        let p3 = p2 + gradient.yx().scale_xy(Point2DF32::new(width, -width));
        let p4 = p3 - offset;

        contour.push_endpoint(p2);
        contour.push_endpoint(p3);
        contour.push_endpoint(p4);
    }
}

struct ContourStrokeToFill {
    input: Contour,
    output: Contour,
    radius: f32,
}

impl ContourStrokeToFill {
    #[inline]
    fn new(input: Contour, output: Contour, radius: f32) -> ContourStrokeToFill {
        ContourStrokeToFill {
            input,
            output,
            radius,
        }
    }

    fn offset_forward(&mut self) {
        for segment in self.input.iter() {
            segment.offset(self.radius, &mut self.output);
        }
    }

    fn offset_backward(&mut self) {
        let mut segments: Vec<_> = self
            .input
            .iter()
            .map(|segment| segment.reversed())
            .collect();
        segments.reverse();
        for segment in &segments {
            segment.offset(self.radius, &mut self.output);
        }
    }
}

trait Offset {
    fn offset(&self, distance: f32, contour: &mut Contour);
    fn offset_once(&self, distance: f32) -> Self;
    fn error_is_within_tolerance(&self, other: &Segment, distance: f32) -> bool;
}

impl Offset for Segment {
    fn offset(&self, distance: f32, contour: &mut Contour) {
        if self.baseline.square_length() < TOLERANCE * TOLERANCE {
            contour.push_full_segment(self, true);
            return;
        }

        let candidate = self.offset_once(distance);
        if self.error_is_within_tolerance(&candidate, distance) {
            contour.push_full_segment(&candidate, true);
            return;
        }

        debug!("--- SPLITTING ---");
        debug!("... PRE-SPLIT: {:?}", self);
        let (before, after) = self.split(0.5);
        debug!("... AFTER-SPLIT: {:?} {:?}", before, after);
        before.offset(distance, contour);
        after.offset(distance, contour);
    }

    fn offset_once(&self, distance: f32) -> Segment {
        if self.is_line() {
            return Segment::line(&self.baseline.offset(distance));
        }

        if self.is_quadratic() {
            let mut segment_0 = LineSegmentF32::new(self.baseline.from(), self.ctrl.from());
            let mut segment_1 = LineSegmentF32::new(self.ctrl.from(), self.baseline.to());
            segment_0 = segment_0.offset(distance);
            segment_1 = segment_1.offset(distance);
            let ctrl = match segment_0.intersection_t(&segment_1) {
                Some(t) => segment_0.sample(t),
                None => segment_0.to().lerp(segment_1.from(), 0.5),
            };
            let baseline = LineSegmentF32::new(segment_0.from(), segment_1.to());
            return Segment::quadratic(&baseline, ctrl);
        }

        debug_assert!(self.is_cubic());

        if self.baseline.from() == self.ctrl.from() {
            let mut segment_0 = LineSegmentF32::new(self.baseline.from(), self.ctrl.to());
            let mut segment_1 = LineSegmentF32::new(self.ctrl.to(), self.baseline.to());
            segment_0 = segment_0.offset(distance);
            segment_1 = segment_1.offset(distance);
            let ctrl = match segment_0.intersection_t(&segment_1) {
                Some(t) => segment_0.sample(t),
                None => segment_0.to().lerp(segment_1.from(), 0.5),
            };
            let baseline = LineSegmentF32::new(segment_0.from(), segment_1.to());
            let ctrl = LineSegmentF32::new(segment_0.from(), ctrl);
            return Segment::cubic(&baseline, &ctrl);
        }

        if self.ctrl.to() == self.baseline.to() {
            let mut segment_0 = LineSegmentF32::new(self.baseline.from(), self.ctrl.from());
            let mut segment_1 = LineSegmentF32::new(self.ctrl.from(), self.baseline.to());
            segment_0 = segment_0.offset(distance);
            segment_1 = segment_1.offset(distance);
            let ctrl = match segment_0.intersection_t(&segment_1) {
                Some(t) => segment_0.sample(t),
                None => segment_0.to().lerp(segment_1.from(), 0.5),
            };
            let baseline = LineSegmentF32::new(segment_0.from(), segment_1.to());
            let ctrl = LineSegmentF32::new(ctrl, segment_1.to());
            return Segment::cubic(&baseline, &ctrl);
        }

        let mut segment_0 = LineSegmentF32::new(self.baseline.from(), self.ctrl.from());
        let mut segment_1 = LineSegmentF32::new(self.ctrl.from(), self.ctrl.to());
        let mut segment_2 = LineSegmentF32::new(self.ctrl.to(), self.baseline.to());
        segment_0 = segment_0.offset(distance);
        segment_1 = segment_1.offset(distance);
        segment_2 = segment_2.offset(distance);
        let (ctrl_0, ctrl_1) = match (
            segment_0.intersection_t(&segment_1),
            segment_1.intersection_t(&segment_2),
        ) {
            (Some(t0), Some(t1)) => (segment_0.sample(t0), segment_1.sample(t1)),
            _ => (
                segment_0.to().lerp(segment_1.from(), 0.5),
                segment_1.to().lerp(segment_2.from(), 0.5),
            ),
        };
        let baseline = LineSegmentF32::new(segment_0.from(), segment_2.to());
        let ctrl = LineSegmentF32::new(ctrl_0, ctrl_1);
        Segment::cubic(&baseline, &ctrl)
    }

    fn error_is_within_tolerance(&self, other: &Segment, distance: f32) -> bool {
        let (mut min, mut max) = (
            f32::abs(distance) - TOLERANCE,
            f32::abs(distance) + TOLERANCE,
        );
        min = if min <= 0.0 { 0.0 } else { min * min };
        max = if max <= 0.0 { 0.0 } else { max * max };

        for t_num in 0..(SAMPLE_COUNT + 1) {
            let t = t_num as f32 / SAMPLE_COUNT as f32;
            // FIXME(pcwalton): Use signed distance!
            let (this_p, other_p) = (self.sample(t), other.sample(t));
            let vector = this_p - other_p;
            let square_distance = vector.square_length();
            debug!(
                "this_p={:?} other_p={:?} vector={:?} sqdist={:?} min={:?} max={:?}",
                this_p, other_p, vector, square_distance, min, max
            );
            if square_distance < min || square_distance > max {
                return false;
            }
        }

        return true;

        const SAMPLE_COUNT: u32 = 16;
    }
}

impl Default for StrokeStyle {
    #[inline]
    fn default() -> StrokeStyle {
        StrokeStyle { line_width: 1.0, line_cap: LineCap::default() }
    }
}

impl Default for LineCap {
    #[inline]
    fn default() -> LineCap { LineCap::Butt }
}

//! An orbit camera, in the cloud's local frame.
//!
//! Up is `+Z`. Every format this application will read — LAS, E57, and the
//! scanners behind them — is Z-up, and a viewer that quietly disagrees with
//! its data turns every scene on its side.
//!
//! The camera works in **local** coordinates: `f32` offsets from the
//! cloud's `f64` origin, exactly as they are stored and exactly as they go
//! to the GPU. A camera in absolute coordinates would put a georeferenced
//! scene half a million metres from the eye and spend the whole `f32`
//! mantissa getting back.

use rigidity_core::nalgebra as na;

/// Where the eye is and what it can see.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Camera {
    /// What the camera orbits, in local coordinates.
    pub(crate) target: na::Point3<f32>,
    /// How far the eye is from it.
    pub(crate) distance: f32,
    /// Angle about `+Z`, radians.
    pub(crate) azimuth: f32,
    /// Angle above the horizon, radians, clamped short of the poles.
    pub(crate) elevation: f32,
    /// Vertical field of view, radians.
    pub(crate) fov_y: f32,
    /// Near plane, metres. There is no far plane; see [`Self::projection`].
    pub(crate) near: f32,
}

/// Stops the view direction from becoming parallel to the up axis, where
/// the frame is undefined and the image flips.
const ELEVATION_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.01;

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: na::Point3::origin(),
            distance: 10.0,
            azimuth: -std::f32::consts::FRAC_PI_4,
            elevation: 0.35,
            fov_y: 50f32.to_radians(),
            near: 0.01,
        }
    }
}

impl Camera {
    /// Where the eye sits.
    pub(crate) fn eye(&self) -> na::Point3<f32> {
        let (sin_e, cos_e) = self.elevation.sin_cos();
        let (sin_a, cos_a) = self.azimuth.sin_cos();
        self.target + self.distance * na::Vector3::new(cos_e * cos_a, cos_e * sin_a, sin_e)
    }

    /// The projection: right-handed, reverse-Z, infinite far plane.
    ///
    /// Reverse-Z spends `f32` depth precision where it is needed. The
    /// familiar `[0, 1]` mapping crowds almost every representable value
    /// against the near plane, and a survey scene spanning four orders of
    /// magnitude then z-fights at its far end. Mapping near to 1 and
    /// infinity to 0 puts the dense end of the floating-point range where
    /// the geometry is. The depth test is `Greater` and the buffer clears
    /// to 0 to match.
    ///
    /// Dropping the far plane entirely costs nothing here and removes a
    /// setting nobody could choose correctly for an unseen scene.
    pub(crate) fn projection(&self, aspect: f32) -> na::Matrix4<f32> {
        let focal = 1.0 / (self.fov_y * 0.5).tan();
        na::Matrix4::new(
            focal / aspect,
            0.0,
            0.0,
            0.0,
            0.0,
            focal,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            self.near,
            0.0,
            0.0,
            -1.0,
            0.0,
        )
    }

    /// World to clip.
    pub(crate) fn view_projection(&self, aspect: f32) -> na::Matrix4<f32> {
        let view = na::Matrix4::look_at_rh(&self.eye(), &self.target, &na::Vector3::z());
        self.projection(aspect) * view
    }

    /// Turns the camera around its target. Input is in points of cursor travel.
    pub(crate) fn orbit(&mut self, delta: [f32; 2]) {
        const RADIANS_PER_POINT: f32 = 0.008;
        self.azimuth -= delta[0] * RADIANS_PER_POINT;
        self.elevation = (self.elevation + delta[1] * RADIANS_PER_POINT)
            .clamp(-ELEVATION_LIMIT, ELEVATION_LIMIT);
    }

    /// Slides the target across the view plane.
    ///
    /// The scale is chosen so that a point under the cursor stays under the
    /// cursor: at the target's depth, one point of cursor travel is one
    /// point of scene travel. Anything else feels like dragging through
    /// treacle at one distance and ice at another.
    pub(crate) fn pan(&mut self, delta: [f32; 2], viewport_height: f32) {
        if viewport_height <= 0.0 {
            return;
        }
        let world_per_point = 2.0 * self.distance * (self.fov_y * 0.5).tan() / viewport_height;
        let forward = (self.target - self.eye()).normalize();
        let right = forward.cross(&na::Vector3::z()).normalize();
        let up = right.cross(&forward);
        self.target += right * (-delta[0] * world_per_point) + up * (delta[1] * world_per_point);
    }

    /// Moves the eye in or out. One notch is a fixed fraction of the
    /// current distance, so the approach is smooth at every scale and
    /// never reaches zero.
    pub(crate) fn dolly(&mut self, notches: f32) {
        self.distance = (self.distance * (-notches * 0.12).exp()).clamp(1e-4, 1e9);
    }

    /// Moves the eye in or out by a ratio rather than by notches.
    ///
    /// A trackpad reports a pinch as the proportion the fingers moved
    /// apart, and passing that straight through is what makes the gesture
    /// track the fingers: half the pinch, half the change, at every
    /// distance. The clamp is against a single absurd event, not against
    /// the gesture.
    pub(crate) fn pinch(&mut self, ratio: f32) {
        self.distance = (self.distance / ratio.clamp(0.1, 10.0)).clamp(1e-4, 1e9);
    }

    /// Frames a bounding box.
    ///
    /// The box's bounding *sphere* is fitted, not the box itself: fitting
    /// the box means the scene leaves the frame as soon as it is turned,
    /// and a view that has to be re-fitted after every orbit is not a fit.
    pub(crate) fn fit(&mut self, min: na::Point3<f32>, max: na::Point3<f32>) {
        let centre = na::center(&min, &max);
        let radius = ((max - min).norm() * 0.5).max(1e-3);
        self.target = centre;
        self.distance = radius / (self.fov_y * 0.5).sin() * 1.05;
        self.near = (radius * 1e-4).max(1e-4);
    }
}

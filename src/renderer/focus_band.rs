//! One horizontal band retains full opacity; the rest of a layer recedes.

use super::ShaderEffect;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Uniform {
    top: f32,
    bottom: f32,
    feather: f32,
    opacity: f32,
}

impl Uniform {
    pub(super) fn new(effect: &ShaderEffect, scale: f32) -> Self {
        let ShaderEffect::FocusBand {
            top,
            bottom,
            feather,
            opacity,
        } = *effect
        else {
            unreachable!("FocusBand uniforms require a FocusBand effect");
        };
        Self {
            top: top * scale,
            bottom: bottom.max(top) * scale,
            feather: feather.max(0.001) * scale,
            opacity: opacity.clamp(0.0, 1.0),
        }
    }
}

pub(super) const SHADER: &str = r#"
struct Params {
    top: f32,
    bottom: f32,
    feather: f32,
    opacity: f32,
};
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>) -> VsOut {
    var out: VsOut;
    out.clip_pos = vec4<f32>(pos, 0.0, 1.0);
    out.uv = uv;
    return out;
}
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let distance = max(params.top - in.clip_pos.y, in.clip_pos.y - params.bottom);
    let outside = smoothstep(0.0, params.feather, distance);
    // The input is premultiplied: color and alpha must fade together.
    return textureSampleLevel(tex, samp, in.uv, 0.0) * mix(1.0, params.opacity, outside);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_band_geometry_without_scaling_opacity() {
        let u = Uniform::new(
            &ShaderEffect::FocusBand {
                top: 200.0,
                bottom: 230.0,
                feather: 5.0,
                opacity: 0.3,
            },
            1.5,
        );
        assert_eq!(
            (u.top, u.bottom, u.feather, u.opacity),
            (300.0, 345.0, 7.5, 0.3)
        );
        assert_eq!(std::mem::size_of::<Uniform>(), 16);
    }
}

//! 数学曲线 loading 动画集。移植自
//! <https://paidax01.github.io/math-curve-loaders/>(原站是 JS + SVG,这里用
//! iced 0.14 的 canvas 纯 Rust 重画,底层走 wgpu/GPU 光栅化,不引任何 JS)。
//!
//! 每条曲线都是一段参数方程:一堆粒子沿曲线排成一条带尾迹的轨迹,叠加一个
//! 随时间"呼吸"的细节振幅(`detail_scale`)和可选的整组慢旋转。原站的 22 个
//! 条目里有两组其实是同一族曲线换参数(5/7/9 瓣玫瑰、R=3..6 的螺旋花、k=2
//! ..5 的玫瑰曲线),这里用同一个 `Curve` 枚举的 21 个变体 + 静态参数表忠实
//! 复刻全部条目。
//!
//! # 动画驱动(纯 Rust,不用 JS)
//!
//! 组件本身是纯函数 `view(curve, elapsed, size)`,只按 `elapsed` 渲染一帧;
//! 连续动画由宿主用 `subscription()` 挂一个时间订阅驱动(iced 的标准做法,同
//! iced 官方 clock 示例)。`subscription()` 内部就是 `iced_futures::time::
//! every(frame_interval())`——原生 Rust 的 tokio 定时器,与 JS 的
//! `requestAnimationFrame` 无关。

use std::time::Duration;

use iced_widget::canvas::{self, Canvas, Geometry};
use iced_widget::core::{Color, Element, Length, Point};
use iced_widget::core::{Rectangle, mouse, window};
use iced_widget::{column, container, text};

/// 建议的动画帧间隔(~60fps)。宿主拿它去 `subscription()` 或自己定节奏。
pub fn frame_interval() -> Duration {
    Duration::from_millis(16)
}

/// 返回一个按 [`frame_interval`] 周期性产生消息的订阅,`f` 把每个 tick 的
/// `Instant` 映射成宿主的 `Message`。典型用法:
///
/// ```ignore
/// fn subscription(&self) -> iced::Subscription<Message> {
///     byteui::feedback::math_curve::subscription(|_| Message::Tick)
/// }
/// ```
///
/// 注意:返回类型是 `iced_futures::Subscription`,即 `iced::Subscription` 的
/// 同一个类型,宿主可以直接 `+`/`batch` 合并进自己的订阅集合。
pub fn subscription<Message: 'static>(
    f: impl Fn(std::time::Instant) -> Message + Send + Clone + 'static,
) -> iced_futures::Subscription<Message> {
    iced_futures::backend::default::time::every(frame_interval()).map(f)
}

/// 全部 21 条曲线。变体名即原站条目名(去空格/连字符)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Curve {
    OriginalThinking,
    ThinkingFive,
    ThinkingNine,
    RoseOrbit,
    RoseCurve,
    RoseTwo,
    RoseThree,
    RoseFour,
    LissajousDrift,
    LemniscateBloom,
    HypotrochoidLoop,
    ThreePetalSpiral,
    FourPetalSpiral,
    FivePetalSpiral,
    SixPetalSpiral,
    ButterflyPhase,
    CardioidGlow,
    CardioidHeart,
    HeartWave,
    SpiralSearch,
    FourierFlow,
}

impl Curve {
    /// 全部曲线,供调用方(如未来的画廊页)枚举。
    pub const ALL: &'static [Curve] = &[
        Curve::OriginalThinking,
        Curve::ThinkingFive,
        Curve::ThinkingNine,
        Curve::RoseOrbit,
        Curve::RoseCurve,
        Curve::RoseTwo,
        Curve::RoseThree,
        Curve::RoseFour,
        Curve::LissajousDrift,
        Curve::LemniscateBloom,
        Curve::HypotrochoidLoop,
        Curve::ThreePetalSpiral,
        Curve::FourPetalSpiral,
        Curve::FivePetalSpiral,
        Curve::SixPetalSpiral,
        Curve::ButterflyPhase,
        Curve::CardioidGlow,
        Curve::CardioidHeart,
        Curve::HeartWave,
        Curve::SpiralSearch,
        Curve::FourierFlow,
    ];

    /// 原站条目名。
    pub fn name(&self) -> &'static str {
        self.spec().name
    }

    fn spec(&self) -> CurveSpec {
        use Curve as C;
        use Family as F;
        let rose = Params {
            rose_a: 9.2,
            rose_a_boost: 0.6,
            rose_breath_base: 0.72,
            rose_breath_boost: 0.28,
            rose_scale: 3.25,
            ..Params::default()
        };
        let spiral = |r: f32| Params {
            spiral_r: r,
            spiral_rr: 1.0,
            spiral_d: 3.0,
            spiral_scale: 2.2,
            spiral_breath: 0.45,
            ..Params::default()
        };
        macro_rules! meta {
            ($name:literal, $family:expr, $params:expr, $rotate:expr, $n:expr, $trail:expr, $dur:expr, $rot_dur:expr, $pulse:expr, $stroke:expr) => {
                CurveSpec {
                    name: $name,
                    family: $family,
                    params: $params,
                    rotate: $rotate,
                    particle_count: $n,
                    trail_span: $trail,
                    duration_ms: $dur,
                    rotation_duration_ms: $rot_dur,
                    pulse_duration_ms: $pulse,
                    stroke_width: $stroke,
                }
            };
        }
        match self {
            C::OriginalThinking => meta!(
                "Original Thinking",
                F::RoseTrail,
                Params {
                    base_radius: 7.0,
                    detail_amplitude: 3.0,
                    petal_count: 7.0,
                    curve_scale: 3.9,
                    ..Params::default()
                },
                true,
                64,
                0.38,
                4600.0,
                28000.0,
                4200.0,
                5.5
            ),
            C::ThinkingFive => meta!(
                "Thinking Five",
                F::RoseTrail,
                Params {
                    base_radius: 7.0,
                    detail_amplitude: 3.0,
                    petal_count: 5.0,
                    curve_scale: 3.9,
                    ..Params::default()
                },
                true,
                62,
                0.38,
                4600.0,
                28000.0,
                4200.0,
                5.5
            ),
            C::ThinkingNine => meta!(
                "Thinking Nine",
                F::RoseTrail,
                Params {
                    base_radius: 7.0,
                    detail_amplitude: 3.0,
                    petal_count: 9.0,
                    curve_scale: 3.9,
                    ..Params::default()
                },
                true,
                68,
                0.39,
                4700.0,
                30000.0,
                4200.0,
                5.5
            ),
            C::RoseOrbit => meta!(
                "Rose Orbit",
                F::RoseOrbit,
                Params {
                    orbit_radius: 7.0,
                    detail_amplitude: 2.7,
                    petal_count: 7.0,
                    curve_scale: 3.9,
                    ..Params::default()
                },
                true,
                72,
                0.42,
                5200.0,
                28000.0,
                4600.0,
                5.2
            ),
            C::RoseCurve => meta!(
                "Rose Curve",
                F::RoseCurve,
                Params {
                    rose_k: 5.0,
                    ..rose
                },
                true,
                78,
                0.32,
                5400.0,
                28000.0,
                4600.0,
                4.5
            ),
            C::RoseTwo => meta!(
                "Rose Two",
                F::RoseCurve,
                Params {
                    rose_k: 2.0,
                    ..rose
                },
                true,
                74,
                0.30,
                5200.0,
                28000.0,
                4300.0,
                4.6
            ),
            C::RoseThree => meta!(
                "Rose Three",
                F::RoseCurve,
                Params {
                    rose_k: 3.0,
                    ..rose
                },
                true,
                76,
                0.31,
                5300.0,
                28000.0,
                4400.0,
                4.6
            ),
            C::RoseFour => meta!(
                "Rose Four",
                F::RoseCurve,
                Params {
                    rose_k: 4.0,
                    ..rose
                },
                true,
                78,
                0.32,
                5400.0,
                28000.0,
                4500.0,
                4.6
            ),
            C::LissajousDrift => meta!(
                "Lissajous Drift",
                F::Lissajous,
                Params {
                    lissajous_amp: 24.0,
                    lissajous_amp_boost: 6.0,
                    lissajous_a: 3.0,
                    lissajous_b: 4.0,
                    lissajous_phase: 1.57,
                    lissajous_y_scale: 0.92,
                    ..Params::default()
                },
                false,
                68,
                0.34,
                6000.0,
                36000.0,
                5400.0,
                4.7
            ),
            C::LemniscateBloom => meta!(
                "Lemniscate Bloom",
                F::Lemniscate,
                Params {
                    lemniscate_a: 20.0,
                    lemniscate_boost: 7.0,
                    ..Params::default()
                },
                false,
                70,
                0.40,
                5600.0,
                34000.0,
                5000.0,
                4.8
            ),
            C::HypotrochoidLoop => meta!(
                "Hypotrochoid Loop",
                F::Hypotrochoid,
                Params {
                    spiro_r: 8.2,
                    spiro_rr: 2.7,
                    spiro_r_boost: 0.45,
                    spiro_d: 4.8,
                    spiro_d_boost: 1.2,
                    spiro_scale: 3.05,
                    ..Params::default()
                },
                false,
                82,
                0.46,
                7600.0,
                42000.0,
                6200.0,
                4.6
            ),
            C::ThreePetalSpiral => meta!(
                "Three-Petal Spiral",
                F::PetalSpiral,
                spiral(3.0),
                true,
                82,
                0.34,
                4600.0,
                28000.0,
                4200.0,
                4.4
            ),
            C::FourPetalSpiral => meta!(
                "Four-Petal Spiral",
                F::PetalSpiral,
                spiral(4.0),
                true,
                84,
                0.34,
                4600.0,
                28000.0,
                4200.0,
                4.4
            ),
            C::FivePetalSpiral => meta!(
                "Five-Petal Spiral",
                F::PetalSpiral,
                spiral(5.0),
                true,
                85,
                0.34,
                4600.0,
                28000.0,
                4200.0,
                4.4
            ),
            C::SixPetalSpiral => meta!(
                "Six-Petal Spiral",
                F::PetalSpiral,
                spiral(6.0),
                true,
                86,
                0.34,
                4600.0,
                28000.0,
                4200.0,
                4.4
            ),
            C::ButterflyPhase => meta!(
                "Butterfly Phase",
                F::Butterfly,
                Params {
                    butterfly_turns: 12.0,
                    butterfly_scale: 4.6,
                    butterfly_pulse: 0.45,
                    butterfly_cos_weight: 2.0,
                    butterfly_power: 5.0,
                    ..Params::default()
                },
                false,
                88,
                0.32,
                9000.0,
                50000.0,
                7000.0,
                4.4
            ),
            C::CardioidGlow => meta!(
                "Cardioid Glow",
                F::CardioidGlow,
                Params {
                    cardioid_a: 8.4,
                    cardioid_pulse: 0.8,
                    cardioid_scale: 2.15,
                    ..Params::default()
                },
                false,
                72,
                0.36,
                6200.0,
                36000.0,
                5200.0,
                4.9
            ),
            C::CardioidHeart => meta!(
                "Cardioid Heart",
                F::CardioidHeart,
                Params {
                    cardioid_a: 8.8,
                    cardioid_pulse: 0.8,
                    cardioid_scale: 2.15,
                    ..Params::default()
                },
                false,
                74,
                0.36,
                6200.0,
                36000.0,
                5200.0,
                4.9
            ),
            C::HeartWave => meta!(
                "Heart Wave",
                F::HeartWave,
                Params {
                    heart_wave_b: 6.4,
                    heart_wave_root: 3.3,
                    heart_wave_amp: 0.9,
                    heart_wave_scale_x: 23.2,
                    heart_wave_scale_y: 24.5,
                    ..Params::default()
                },
                false,
                104,
                0.18,
                8400.0,
                22000.0,
                5600.0,
                3.9
            ),
            C::SpiralSearch => meta!(
                "Spiral Search",
                F::SpiralSearch,
                Params {
                    search_turns: 4.0,
                    search_base_radius: 8.0,
                    search_radius_amp: 8.5,
                    search_pulse: 2.4,
                    search_scale: 1.0,
                    ..Params::default()
                },
                false,
                86,
                0.28,
                7800.0,
                44000.0,
                6800.0,
                4.3
            ),
            C::FourierFlow => meta!(
                "Fourier Flow",
                F::Fourier,
                Params {
                    fourier_x1: 17.0,
                    fourier_x3: 7.5,
                    fourier_x5: 3.2,
                    fourier_y1: 15.0,
                    fourier_y2: 8.2,
                    fourier_y4: 4.2,
                    fourier_mix_base: 1.0,
                    fourier_mix_pulse: 0.16,
                    ..Params::default()
                },
                false,
                92,
                0.31,
                8400.0,
                44000.0,
                6800.0,
                4.2
            ),
        }
    }
}

/// 曲线族(决定 `point` 用哪条参数方程)。`Params` 是各族参数的并集,每个
/// 变体只用其中若干字段,其余留 `0.0`。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    RoseTrail,
    RoseOrbit,
    RoseCurve,
    Lissajous,
    Lemniscate,
    Hypotrochoid,
    PetalSpiral,
    Butterfly,
    CardioidGlow,
    CardioidHeart,
    HeartWave,
    SpiralSearch,
    Fourier,
}

#[derive(Clone, Copy, Default)]
struct Params {
    // RoseTrail / RoseOrbit 共用
    base_radius: f32,
    orbit_radius: f32,
    detail_amplitude: f32,
    petal_count: f32,
    curve_scale: f32,
    // RoseCurve
    rose_a: f32,
    rose_a_boost: f32,
    rose_breath_base: f32,
    rose_breath_boost: f32,
    rose_k: f32,
    rose_scale: f32,
    // Lissajous
    lissajous_amp: f32,
    lissajous_amp_boost: f32,
    lissajous_a: f32,
    lissajous_b: f32,
    lissajous_phase: f32,
    lissajous_y_scale: f32,
    // Lemniscate
    lemniscate_a: f32,
    lemniscate_boost: f32,
    // Hypotrochoid
    spiro_r: f32,
    spiro_rr: f32,
    spiro_r_boost: f32,
    spiro_d: f32,
    spiro_d_boost: f32,
    spiro_scale: f32,
    // PetalSpiral
    spiral_r: f32,
    spiral_rr: f32,
    spiral_d: f32,
    spiral_scale: f32,
    spiral_breath: f32,
    // Butterfly
    butterfly_turns: f32,
    butterfly_scale: f32,
    butterfly_pulse: f32,
    butterfly_cos_weight: f32,
    butterfly_power: f32,
    // Cardioid
    cardioid_a: f32,
    cardioid_pulse: f32,
    cardioid_scale: f32,
    // HeartWave
    heart_wave_b: f32,
    heart_wave_root: f32,
    heart_wave_amp: f32,
    heart_wave_scale_x: f32,
    heart_wave_scale_y: f32,
    // SpiralSearch
    search_turns: f32,
    search_base_radius: f32,
    search_radius_amp: f32,
    search_pulse: f32,
    search_scale: f32,
    // Fourier
    fourier_x1: f32,
    fourier_x3: f32,
    fourier_x5: f32,
    fourier_y1: f32,
    fourier_y2: f32,
    fourier_y4: f32,
    fourier_mix_base: f32,
    fourier_mix_pulse: f32,
}

#[derive(Clone, Copy)]
struct CurveSpec {
    name: &'static str,
    family: Family,
    params: Params,
    rotate: bool,
    particle_count: usize,
    trail_span: f32,
    duration_ms: f32,
    rotation_duration_ms: f32,
    pulse_duration_ms: f32,
    stroke_width: f32,
}

impl CurveSpec {
    /// 参数方程取值,返回 0..=100 坐标系下的点(原站 SVG 的 `viewBox
    /// 0 0 100 100`)。`detail` 是随时间呼吸的细节振幅(`s`),由
    /// [`detail_scale`] 产出。
    fn point(&self, progress: f32, detail: f32) -> Point {
        use std::f32::consts::{PI, TAU};
        let p = &self.params;
        match self.family {
            Family::RoseTrail => {
                let t = progress * TAU;
                let petals = p.petal_count.round();
                let x = p.base_radius * t.cos() - p.detail_amplitude * detail * (petals * t).cos();
                let y = p.base_radius * t.sin() - p.detail_amplitude * detail * (petals * t).sin();
                Point::new(50.0 + x * p.curve_scale, 50.0 + y * p.curve_scale)
            }
            Family::RoseOrbit => {
                let t = progress * TAU;
                let k = p.petal_count.round();
                let r = p.orbit_radius - p.detail_amplitude * detail * (k * t).cos();
                Point::new(
                    50.0 + t.cos() * r * p.curve_scale,
                    50.0 + t.sin() * r * p.curve_scale,
                )
            }
            Family::RoseCurve => {
                let t = progress * TAU;
                let a = p.rose_a + detail * p.rose_a_boost;
                let k = p.rose_k.round();
                let r = a * (p.rose_breath_base + detail * p.rose_breath_boost) * (k * t).cos();
                Point::new(
                    50.0 + t.cos() * r * p.rose_scale,
                    50.0 + t.sin() * r * p.rose_scale,
                )
            }
            Family::Lissajous => {
                let t = progress * TAU;
                let amp = p.lissajous_amp + detail * p.lissajous_amp_boost;
                let x = 50.0 + (p.lissajous_a.round() * t + p.lissajous_phase).sin() * amp;
                let y = 50.0 + (p.lissajous_b.round() * t).sin() * (amp * p.lissajous_y_scale);
                Point::new(x, y)
            }
            Family::Lemniscate => {
                let t = progress * TAU;
                let scale = p.lemniscate_a + detail * p.lemniscate_boost;
                let denom = 1.0 + t.sin().powi(2);
                Point::new(
                    50.0 + scale * t.cos() / denom,
                    50.0 + scale * t.sin() * t.cos() / denom,
                )
            }
            Family::Hypotrochoid => {
                let t = progress * TAU;
                let r = p.spiro_rr + detail * p.spiro_r_boost;
                let d = p.spiro_d + detail * p.spiro_d_boost;
                let rr = p.spiro_r - r;
                let x = 50.0 + (rr * t.cos() + d * (rr * t / r).cos()) * p.spiro_scale;
                let y = 50.0 + (rr * t.sin() - d * (rr * t / r).sin()) * p.spiro_scale;
                Point::new(x, y)
            }
            Family::PetalSpiral => {
                let t = progress * TAU;
                let d = p.spiral_d + detail * 0.25;
                let rr = p.spiral_r - p.spiral_rr;
                let base_x = rr * t.cos() + d * (rr * t / p.spiral_rr).cos();
                let base_y = rr * t.sin() - d * (rr * t / p.spiral_rr).sin();
                let scale = p.spiral_scale + detail * p.spiral_breath;
                Point::new(50.0 + base_x * scale, 50.0 + base_y * scale)
            }
            Family::Butterfly => {
                let t = progress * PI * p.butterfly_turns;
                let b = t.cos().exp()
                    - p.butterfly_cos_weight * (4.0 * t).cos()
                    - (t / 12.0).sin().powi(p.butterfly_power.round() as i32);
                let scale = p.butterfly_scale + detail * p.butterfly_pulse;
                Point::new(50.0 + t.sin() * b * scale, 50.0 + t.cos() * b * scale)
            }
            Family::CardioidGlow => {
                let t = progress * TAU;
                let a = p.cardioid_a + detail * p.cardioid_pulse;
                let r = a * (1.0 - t.cos());
                Point::new(
                    50.0 + t.cos() * r * p.cardioid_scale,
                    50.0 + t.sin() * r * p.cardioid_scale,
                )
            }
            Family::CardioidHeart => {
                let t = progress * TAU;
                let a = p.cardioid_a + detail * p.cardioid_pulse;
                let r = a * (1.0 + t.cos());
                let base_x = t.cos() * r;
                let base_y = t.sin() * r;
                Point::new(
                    50.0 - base_y * p.cardioid_scale,
                    50.0 - base_x * p.cardioid_scale,
                )
            }
            Family::HeartWave => {
                let x_limit = p.heart_wave_root.sqrt();
                let x = -x_limit + progress * x_limit * 2.0;
                let safe_root = (p.heart_wave_root - x * x).max(0.0);
                let wave = p.heart_wave_amp * safe_root.sqrt() * (p.heart_wave_b * PI * x).sin();
                let curve = x.abs().powf(2.0 / 3.0);
                let y = curve + wave;
                let scale_y = p.heart_wave_scale_y + detail * 1.5;
                Point::new(50.0 + x * p.heart_wave_scale_x, 18.0 + (1.75 - y) * scale_y)
            }
            Family::SpiralSearch => {
                let t = progress * TAU;
                let angle = t * p.search_turns;
                let radius = p.search_base_radius
                    + (1.0 - t.cos()) * (p.search_radius_amp + detail * p.search_pulse);
                Point::new(
                    50.0 + angle.cos() * radius * p.search_scale,
                    50.0 + angle.sin() * radius * p.search_scale,
                )
            }
            Family::Fourier => {
                let t = progress * TAU;
                let mix = p.fourier_mix_base + detail * p.fourier_mix_pulse;
                let x = p.fourier_x1 * t.cos()
                    + p.fourier_x3 * (3.0 * t + 0.6 * mix).cos()
                    + p.fourier_x5 * (5.0 * t - 0.4).sin();
                let y = p.fourier_y1 * t.sin() + p.fourier_y2 * (2.0 * t + 0.25).sin()
                    - p.fourier_y4 * (4.0 * t - 0.5 * mix).cos();
                Point::new(50.0 + x, 50.0 + y)
            }
        }
    }
}

/// 把进度折叠回 0..1(粒子尾迹往前探会越到负进度,原站 `normalizeProgress`)。
fn normalize(progress: f32) -> f32 {
    ((progress % 1.0) + 1.0) % 1.0
}

/// 随时间呼吸的细节振幅 `s`。原站 `getDetailScale`:正弦包络,取值
/// `0.52..=1.0`。`phase` 是 0..1 的相位偏移(原站用 `Math.random()` 让多个
/// 实例不同步,这里默认 0)。
fn detail_scale(elapsed_ms: f32, pulse_duration_ms: f32, phase: f32) -> f32 {
    let pulse_progress =
        ((elapsed_ms + phase * pulse_duration_ms) % pulse_duration_ms) / pulse_duration_ms;
    let angle = pulse_progress * std::f32::consts::TAU;
    0.52 + ((angle + 0.55).sin() + 1.0) / 2.0 * 0.48
}

/// 整组旋转角度(度),负值 = 逆时针(与原站 `getRotation` 一致)。`rotate` 为
/// 假时恒 0。
fn rotation_deg(elapsed_ms: f32, rotation_duration_ms: f32, rotate: bool, phase: f32) -> f32 {
    if !rotate {
        return 0.0;
    }
    let progress =
        ((elapsed_ms + phase * rotation_duration_ms) % rotation_duration_ms) / rotation_duration_ms;
    -(progress * 360.0)
}

/// 用主题 `cream`(正文色,对应原站的 `currentColor`)渲染 `curve` 的一帧。
/// `elapsed` 是自加载开始经过的时长,宿主每帧传入新值即可(见模块文档)。
pub fn view<'a, Message: 'a>(
    curve: Curve,
    elapsed: Duration,
    size: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    view_with_color(curve, elapsed, size, crate::theme::color::current().cream)
}

/// [`view`] 的自定义颜色版本。
pub fn view_with_color<'a, Message: 'a>(
    curve: Curve,
    elapsed: Duration,
    size: f32,
    color: Color,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    Canvas::new(Loader {
        curve,
        elapsed,
        color,
    })
    .width(Length::Fixed(size))
    .height(Length::Fixed(size))
    .into()
}

/// 自包含的动画版本:不用宿主提供 `elapsed`,组件内部靠 `RedrawRequested`
/// 自驱动连续重绘(iced 官方 `request_redraw_at` 文档即以此做动画,如文本
/// 输入框的光标闪烁)。适合没有订阅/时钟基础设施的宿主(如直接跑 winit
/// 事件循环的 app),一行放进布局即持续动画。只要组件留在树里就持续请求
/// 重绘,移出树即停止。
pub fn view_animated<'a, Message: 'a>(
    curve: Curve,
    size: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    view_animated_with_color(curve, size, crate::theme::color::current().cream)
}

/// [`view_animated`] 的自定义颜色版本。
pub fn view_animated_with_color<'a, Message: 'a>(
    curve: Curve,
    size: f32,
    color: Color,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    Canvas::new(AnimatedLoader { curve, color })
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
}

/// 一个居中的 loading 组合:数学曲线动画 + 一行 `dim` 文案竖排,外层容器
/// `center_x/center_y(Fill)` 让它在宿主给到的空间里水平、垂直都居中。
/// 适合"整块内容区正在异步加载"的场景(用量/搜索/数据库/git log/会话列表
/// 等),一处一行替换掉原来的纯文本「加载中…」。文案与曲线颜色都走当前
/// 主题,`size` 是曲线动画的边长。
pub fn loading_hint<'a, Message: 'a>(
    curve: Curve,
    label: &'static str,
    size: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(
        column![
            view_animated(curve, size),
            text(label)
                .size(crate::theme::font::body())
                .color(crate::theme::color::current().dim),
        ]
        .spacing(12)
        .align_x(iced_widget::core::alignment::Horizontal::Center),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}

struct Loader {
    curve: Curve,
    elapsed: Duration,
    color: Color,
}

impl<Message> canvas::Program<Message, iced_widget::Theme, iced_renderer::Renderer> for Loader {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_renderer::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<iced_renderer::Renderer>> {
        vec![draw_frame(
            renderer,
            bounds,
            self.curve.spec(),
            self.elapsed,
            self.color,
        )]
    }
}

/// [`view_animated`] 的内部 program:起始时间记在 `State` 里(树 diff 时该
/// `State` 会随 canvas 节点保留,`start` 不会被每帧重建),`update` 对每个
/// `RedrawRequested` 再请求一次重绘,形成自维持的动画循环。
struct AnimatedLoader {
    curve: Curve,
    color: Color,
}

struct AnimatedState {
    start: std::time::Instant,
}

impl Default for AnimatedState {
    fn default() -> Self {
        AnimatedState {
            start: std::time::Instant::now(),
        }
    }
}

impl<Message> canvas::Program<Message, iced_widget::Theme, iced_renderer::Renderer>
    for AnimatedLoader
{
    type State = AnimatedState;

    fn update(
        &self,
        _state: &mut Self::State,
        event: &canvas::Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if matches!(
            event,
            canvas::Event::Window(window::Event::RedrawRequested(_))
        ) {
            return Some(canvas::Action::request_redraw());
        }
        None
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &iced_renderer::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<iced_renderer::Renderer>> {
        vec![draw_frame(
            renderer,
            bounds,
            self.curve.spec(),
            state.start.elapsed(),
            self.color,
        )]
    }
}

/// 把一条曲线在给定 `elapsed` 时刻的一帧画进 canvas。`Loader` 与
/// `AnimatedLoader` 共用,只是 `elapsed` 来源不同(前者宿主传入、后者读
/// `State` 的起始时间差)。
fn draw_frame(
    renderer: &iced_renderer::Renderer,
    bounds: Rectangle,
    spec: CurveSpec,
    elapsed: Duration,
    color: Color,
) -> Geometry<iced_renderer::Renderer> {
    let mut frame = canvas::Frame::new(renderer, bounds.size());
    let side = bounds.width.min(bounds.height);
    if side < 1.0 {
        return frame.into_geometry();
    }
    // SVG viewBox 0 0 100 100 -> 画布,居中。
    let k = side / 100.0;
    let center = Point::new(side / 2.0, side / 2.0);

    let elapsed_ms = elapsed.as_secs_f32() * 1000.0;
    let detail = detail_scale(elapsed_ms, spec.pulse_duration_ms, 0.0);
    let rotation = rotation_deg(elapsed_ms, spec.rotation_duration_ms, spec.rotate, 0.0);
    let rot = rotation.to_radians();
    let (cos, sin) = (rot.cos(), rot.sin());

    // 0..100 坐标 -> 画布,并按需绕中心旋转(原站 `rotate(rotation 50 50)`)。
    let map = |p: Point| -> Point {
        let px = p.x * k;
        let py = p.y * k;
        let dx = px - center.x;
        let dy = py - center.y;
        Point::new(
            center.x + dx * cos - dy * sin,
            center.y + dx * sin + dy * cos,
        )
    };

    // 底路径(淡色整条曲线,原站 opacity 0.1;也会随 detail 一起呼吸)。
    let base = Color {
        a: color.a * 0.1,
        ..color
    };
    let steps = 480;
    let path = canvas::Path::new(|b| {
        for i in 0..=steps {
            let q = map(spec.point(i as f32 / steps as f32, detail));
            if i == 0 {
                b.move_to(q);
            } else {
                b.line_to(q);
            }
        }
    });
    frame.stroke(
        &path,
        canvas::Stroke::default()
            .with_color(base)
            .with_width(spec.stroke_width * k),
    );

    // 粒子尾迹(原站 getParticle)。
    let progress = (elapsed_ms % spec.duration_ms) / spec.duration_ms;
    let n = spec.particle_count;
    for i in 0..n {
        let tail_offset = i as f32 / (n - 1).max(1) as f32;
        let p = map(spec.point(normalize(progress - tail_offset * spec.trail_span), detail));
        let fade = (1.0 - tail_offset).powf(0.56);
        let radius = (0.9 + fade * 2.7) * k;
        let opacity = 0.04 + fade * 0.96;
        frame.fill(
            &canvas::Path::circle(p, radius),
            Color {
                a: color.a * opacity,
                ..color
            },
        );
    }

    frame.into_geometry()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(a: f32, b: f32, eps: f32) {
        assert!((a - b).abs() <= eps, "expected {a} ≈ {b} (±{eps})");
    }

    #[test]
    fn has_all_21_curves_with_names() {
        assert_eq!(Curve::ALL.len(), 21);
        for c in Curve::ALL {
            assert!(!c.name().is_empty());
        }
    }

    #[test]
    fn normalize_wraps_into_unit_range() {
        assert_close(normalize(0.0), 0.0, 1e-6);
        assert_close(normalize(0.999), 0.999, 1e-6);
        assert_close(normalize(1.0), 0.0, 1e-6);
        assert_close(normalize(1.5), 0.5, 1e-6);
        assert_close(normalize(-0.5), 0.5, 1e-6);
    }

    #[test]
    fn detail_scale_stays_within_bounds() {
        for ms in (0..1000).map(|i| i as f32 * 37.0) {
            let d = detail_scale(ms, 4200.0, 0.0);
            assert!(
                (0.52 - 1e-6..=1.0 + 1e-6).contains(&d),
                "detail {d} out of range at {ms}"
            );
        }
    }

    #[test]
    fn rotation_is_zero_when_not_rotating() {
        assert_eq!(rotation_deg(1234.0, 28000.0, false, 0.0), 0.0);
    }

    #[test]
    fn rotation_cycles_and_is_negative() {
        // 半程时 -180 度。
        assert_close(rotation_deg(14000.0, 28000.0, true, 0.0), -180.0, 0.01);
    }

    #[test]
    fn every_curve_produces_finite_points_across_sweep() {
        for &c in Curve::ALL {
            let spec = c.spec();
            for i in 0..=10 {
                let progress = i as f32 / 10.0;
                for detail in [0.52, 1.0] {
                    let p = spec.point(progress, detail);
                    assert!(
                        p.x.is_finite() && p.y.is_finite(),
                        "{} NaN/inf at progress={progress} detail={detail}",
                        c.name()
                    );
                }
            }
        }
    }

    #[test]
    fn rose_curve_point_at_progress_zero_matches_formula() {
        // RoseCurve(k=5):t=0 -> cos=1,sin=0;r = a*(breath)*cos(0)=a*breath。
        // a = rose_a + detail*rose_a_boost;breath = base + detail*boost。
        // 取 detail=1.0: a = 9.2+0.6=9.8, breath = 0.72+0.28=1.0, r=9.8。
        // x = 50 + cos(0)*r*scale = 50 + 9.8*3.25;y = 50。
        let spec = Curve::RoseCurve.spec();
        let p = spec.point(0.0, 1.0);
        assert_close(p.x, 50.0 + 9.8 * 3.25, 0.01);
        assert_close(p.y, 50.0, 0.01);
    }

    #[test]
    fn cardioid_glow_point_at_progress_zero_matches_formula() {
        // t=0 -> r = a*(1-cos0)=0 -> 原点(50,50)。
        let spec = Curve::CardioidGlow.spec();
        let p = spec.point(0.0, 1.0);
        assert_close(p.x, 50.0, 0.01);
        assert_close(p.y, 50.0, 0.01);
    }

    #[test]
    fn heart_wave_covers_left_edge_at_progress_zero() {
        // progress=0 -> x = -sqrt(root)。确保 x 分量 < 50(偏左)。
        let spec = Curve::HeartWave.spec();
        let p = spec.point(0.0, 1.0);
        assert!(p.x < 50.0, "expected left-edge x < 50, got {}", p.x);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputState {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Velocity {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementStep {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StructureObstacle {
    pub x: f32,
    pub y: f32,
    pub half_extent: f32,
}

pub const PLAYER_COLLIDER_RADIUS: f32 = 10.0;
pub const STRUCTURE_COLLIDER_HALF_EXTENT: f32 = 11.0;
pub const TERRAIN_GENERATOR_VERSION: u32 = 1;
pub const TERRAIN_TILE_SIZE: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainBaseKind {
    DeepWater,
    ShallowWater,
    Grass,
    Dirt,
    Rock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainResourceKind {
    IronOre,
    CopperOre,
    Coal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainSample {
    pub base: TerrainBaseKind,
    pub resource: Option<TerrainResourceKind>,
    pub resource_richness: u16,
}

pub fn deterministic_seed_from_room_code(room_code: &str) -> u64 {
    // FNV-1a 64-bit hash with uppercased room code for stable seed derivation.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in room_code.trim().to_ascii_uppercase().as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn hash_grid(seed: u64, x: i32, y: i32) -> u64 {
    let x_bits = x as i64 as u64;
    let y_bits = y as i64 as u64;
    splitmix64(
        seed ^ x_bits.wrapping_mul(0x517c_c1b7_2722_0a95)
            ^ y_bits.wrapping_mul(0x9e37_79b9_7f4a_7c15),
    )
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn smoothstep(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}

fn lattice_value(seed: u64, x: i32, y: i32) -> f32 {
    let sample = (hash_grid(seed, x, y) >> 40) as u32;
    (sample as f32 / 16_777_215.0) * 2.0 - 1.0
}

fn value_noise(seed: u64, x: f32, y: f32, frequency: f32) -> f32 {
    let fx = x * frequency;
    let fy = y * frequency;
    let x0 = fx.floor() as i32;
    let y0 = fy.floor() as i32;
    let x1 = x0 + 1;
    let y1 = y0 + 1;
    let tx = smoothstep(fx - x0 as f32);
    let ty = smoothstep(fy - y0 as f32);

    let n00 = lattice_value(seed, x0, y0);
    let n10 = lattice_value(seed, x1, y0);
    let n01 = lattice_value(seed, x0, y1);
    let n11 = lattice_value(seed, x1, y1);

    let nx0 = lerp(n00, n10, tx);
    let nx1 = lerp(n01, n11, tx);
    lerp(nx0, nx1, ty)
}

fn fractal_noise(seed: u64, x: i32, y: i32, base_frequency: f32) -> f32 {
    let mut amplitude = 0.62;
    let mut frequency = base_frequency;
    let mut value = 0.0;
    let mut normalizer = 0.0;

    for octave in 0..4 {
        let octave_seed = splitmix64(seed ^ octave as u64);
        value += value_noise(octave_seed, x as f32, y as f32, frequency) * amplitude;
        normalizer += amplitude;
        amplitude *= 0.52;
        frequency *= 2.03;
    }

    if normalizer <= f32::EPSILON {
        0.0
    } else {
        (value / normalizer).clamp(-1.0, 1.0)
    }
}

pub fn sample_terrain(seed: u64, tile_x: i32, tile_y: i32) -> TerrainSample {
    let elevation = fractal_noise(seed ^ 0x6a09_e667_f3bc_c909, tile_x, tile_y, 0.0175);
    let moisture = fractal_noise(seed ^ 0xbb67_ae85_84ca_a73b, tile_x, tile_y, 0.024);
    let ore_patch = fractal_noise(seed ^ 0x3c6e_f372_fe94_f82b, tile_x, tile_y, 0.031);
    let ore_mix = fractal_noise(seed ^ 0xa54f_f53a_5f1d_36f1, tile_x, tile_y, 0.087);

    let base = if elevation < -0.34 {
        TerrainBaseKind::DeepWater
    } else if elevation < -0.14 {
        TerrainBaseKind::ShallowWater
    } else if elevation > 0.50 {
        TerrainBaseKind::Rock
    } else if moisture < -0.25 {
        TerrainBaseKind::Dirt
    } else {
        TerrainBaseKind::Grass
    };

    let can_host_resources = !matches!(
        base,
        TerrainBaseKind::DeepWater | TerrainBaseKind::ShallowWater
    );
    let resource = if can_host_resources && ore_patch > 0.37 {
        if ore_mix > 0.33 {
            Some(TerrainResourceKind::IronOre)
        } else if ore_mix > -0.08 {
            Some(TerrainResourceKind::CopperOre)
        } else {
            Some(TerrainResourceKind::Coal)
        }
    } else {
        None
    };

    let resource_richness = if resource.is_some() {
        let normalized_patch = (ore_patch - 0.37).max(0.0);
        let normalized_mix = (ore_mix + 1.0) * 0.5;
        let richness = 50.0 + normalized_patch * 900.0 + normalized_mix * 180.0;
        richness.round().clamp(50.0, 1200.0) as u16
    } else {
        0
    };

    TerrainSample {
        base,
        resource,
        resource_richness,
    }
}

pub fn clamp_axis(value: f32, map_limit: f32) -> f32 {
    value.max(-map_limit).min(map_limit)
}

pub fn movement_velocity(input: InputState, speed: f32) -> Velocity {
    let mut dx = 0.0f32;
    let mut dy = 0.0f32;

    if input.up {
        dy += 1.0;
    }
    if input.down {
        dy -= 1.0;
    }
    if input.left {
        dx -= 1.0;
    }
    if input.right {
        dx += 1.0;
    }

    if dx == 0.0 && dy == 0.0 {
        return Velocity { x: 0.0, y: 0.0 };
    }

    let magnitude = (dx * dx + dy * dy).sqrt();
    Velocity {
        x: (dx / magnitude) * speed,
        y: (dy / magnitude) * speed,
    }
}

pub fn movement_step(
    x: f32,
    y: f32,
    input: InputState,
    dt_seconds: f32,
    speed: f32,
    map_limit: f32,
) -> MovementStep {
    movement_step_with_obstacles(
        x,
        y,
        input,
        dt_seconds,
        speed,
        map_limit,
        &[],
        PLAYER_COLLIDER_RADIUS,
    )
}

fn collides_with_obstacle(
    x: f32,
    y: f32,
    obstacle: &StructureObstacle,
    player_radius: f32,
) -> bool {
    let blocked = obstacle.half_extent + player_radius;
    (x - obstacle.x).abs() < blocked && (y - obstacle.y).abs() < blocked
}

pub fn movement_step_with_obstacles(
    x: f32,
    y: f32,
    input: InputState,
    dt_seconds: f32,
    speed: f32,
    map_limit: f32,
    obstacles: &[StructureObstacle],
    player_radius: f32,
) -> MovementStep {
    let velocity = movement_velocity(input, speed);

    let desired_x = clamp_axis(x + velocity.x * dt_seconds, map_limit);
    let desired_y = clamp_axis(y + velocity.y * dt_seconds, map_limit);

    let mut resolved_x = desired_x;
    let mut resolved_y = desired_y;
    let mut resolved_vx = velocity.x;
    let mut resolved_vy = velocity.y;

    if obstacles
        .iter()
        .any(|obstacle| collides_with_obstacle(resolved_x, y, obstacle, player_radius))
    {
        resolved_x = x;
        resolved_vx = 0.0;
    }

    if obstacles
        .iter()
        .any(|obstacle| collides_with_obstacle(resolved_x, resolved_y, obstacle, player_radius))
    {
        resolved_y = y;
        resolved_vy = 0.0;
    }

    MovementStep {
        x: resolved_x,
        y: resolved_y,
        vx: resolved_vx,
        vy: resolved_vy,
    }
}

pub fn projectile_step(
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    dt_seconds: f32,
    map_limit: f32,
) -> (f32, f32) {
    (
        clamp_axis(x + vx * dt_seconds, map_limit),
        clamp_axis(y + vy * dt_seconds, map_limit),
    )
}

#[no_mangle]
pub extern "C" fn sim_compute_velocity_x(
    up: u32,
    down: u32,
    left: u32,
    right: u32,
    speed: f32,
) -> f32 {
    movement_velocity(
        InputState {
            up: up != 0,
            down: down != 0,
            left: left != 0,
            right: right != 0,
        },
        speed,
    )
    .x
}

#[no_mangle]
pub extern "C" fn sim_compute_velocity_y(
    up: u32,
    down: u32,
    left: u32,
    right: u32,
    speed: f32,
) -> f32 {
    movement_velocity(
        InputState {
            up: up != 0,
            down: down != 0,
            left: left != 0,
            right: right != 0,
        },
        speed,
    )
    .y
}

#[no_mangle]
pub extern "C" fn sim_integrate_position(
    position: f32,
    velocity: f32,
    dt_seconds: f32,
    map_limit: f32,
) -> f32 {
    clamp_axis(position + velocity * dt_seconds, map_limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagonal_velocity_is_normalized() {
        let velocity = movement_velocity(
            InputState {
                up: true,
                down: false,
                left: false,
                right: true,
            },
            220.0,
        );

        let magnitude = (velocity.x * velocity.x + velocity.y * velocity.y).sqrt();
        assert!((magnitude - 220.0).abs() < 0.001);
    }

    #[test]
    fn movement_step_clamps_at_bounds() {
        let result = movement_step(
            4999.0,
            0.0,
            InputState {
                up: false,
                down: false,
                left: false,
                right: true,
            },
            1.0,
            220.0,
            5000.0,
        );

        assert_eq!(result.x, 5000.0);
    }

    #[test]
    fn projectile_step_clamps_at_bounds() {
        let (x, y) = projectile_step(0.0, 0.0, 10000.0, -10000.0, 1.0, 5500.0);
        assert_eq!(x, 5500.0);
        assert_eq!(y, -5500.0);
    }

    #[test]
    fn movement_step_respects_obstacles() {
        let obstacle = StructureObstacle {
            x: 0.0,
            y: 0.0,
            half_extent: STRUCTURE_COLLIDER_HALF_EXTENT,
        };

        let result = movement_step_with_obstacles(
            -40.0,
            0.0,
            InputState {
                up: false,
                down: false,
                left: false,
                right: true,
            },
            0.25,
            220.0,
            5000.0,
            &[obstacle],
            PLAYER_COLLIDER_RADIUS,
        );

        assert_eq!(result.x, -40.0);
        assert_eq!(result.vx, 0.0);
    }

    #[test]
    fn deterministic_movement_sequence_matches() {
        let inputs = [
            InputState {
                up: true,
                down: false,
                left: false,
                right: true,
            },
            InputState {
                up: true,
                down: false,
                left: false,
                right: false,
            },
            InputState {
                up: false,
                down: false,
                left: true,
                right: false,
            },
            InputState {
                up: false,
                down: true,
                left: false,
                right: false,
            },
        ];

        let mut a = (0.0f32, 0.0f32);
        let mut b = (0.0f32, 0.0f32);
        for step_index in 0..120 {
            let input = inputs[step_index % inputs.len()];
            let step_a = movement_step(a.0, a.1, input, 1.0 / 60.0, 220.0, 5000.0);
            let step_b = movement_step(b.0, b.1, input, 1.0 / 60.0, 220.0, 5000.0);
            a = (step_a.x, step_a.y);
            b = (step_b.x, step_b.y);
        }

        assert!((a.0 - b.0).abs() < 0.0001);
        assert!((a.1 - b.1).abs() < 0.0001);
    }

    #[test]
    fn deterministic_room_seed_is_case_insensitive() {
        let lower = deterministic_seed_from_room_code("north_hub");
        let upper = deterministic_seed_from_room_code("NORTH_HUB");
        assert_eq!(lower, upper);
    }

    #[test]
    fn terrain_sample_is_deterministic_for_same_seed_and_tile() {
        let seed = deterministic_seed_from_room_code("ALPHA_ROOM");
        let first = sample_terrain(seed, 42, -13);
        let second = sample_terrain(seed, 42, -13);
        assert_eq!(first, second);
    }

    #[test]
    fn terrain_samples_change_with_seed() {
        let seed_a = deterministic_seed_from_room_code("ALPHA_ROOM");
        let seed_b = deterministic_seed_from_room_code("BRAVO_ROOM");

        let mut changed = false;
        for x in -6..=6 {
            for y in -6..=6 {
                if sample_terrain(seed_a, x, y) != sample_terrain(seed_b, x, y) {
                    changed = true;
                    break;
                }
            }
            if changed {
                break;
            }
        }

        assert!(changed);
    }

    #[test]
    fn terrain_resource_richness_is_only_present_with_resource() {
        let seed = deterministic_seed_from_room_code("RESOURCE_TEST");
        for x in -24..=24 {
            for y in -24..=24 {
                let sample = sample_terrain(seed, x, y);
                if sample.resource.is_some() {
                    assert!(sample.resource_richness > 0);
                } else {
                    assert_eq!(sample.resource_richness, 0);
                }
            }
        }
    }
}

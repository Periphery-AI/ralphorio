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
    let velocity = movement_velocity(input, speed);
    MovementStep {
        x: clamp_axis(x + velocity.x * dt_seconds, map_limit),
        y: clamp_axis(y + velocity.y * dt_seconds, map_limit),
        vx: velocity.x,
        vy: velocity.y,
    }
}

pub fn projectile_step(x: f32, y: f32, vx: f32, vy: f32, dt_seconds: f32, map_limit: f32) -> (f32, f32) {
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
pub extern "C" fn sim_integrate_position(position: f32, velocity: f32, dt_seconds: f32, map_limit: f32) -> f32 {
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
}

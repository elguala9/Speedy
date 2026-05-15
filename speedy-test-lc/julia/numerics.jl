"""Numerical methods: Newton–Raphson, bisection, and Runge–Kutta 4."""

# ── Root-finding ──────────────────────────────────────────────────────────────

function newton_raphson(f, df, x0; tol=1e-10, maxiter=100)
    x = x0
    for _ in 1:maxiter
        fx = f(x)
        abs(fx) < tol && return x
        x -= fx / df(x)
    end
    error("Newton–Raphson did not converge")
end

function bisection(f, a, b; tol=1e-10, maxiter=200)
    f(a) * f(b) > 0 && error("f(a) and f(b) must have opposite signs")
    for _ in 1:maxiter
        m = (a + b) / 2
        abs(b - a) < tol && return m
        f(a) * f(m) ≤ 0 ? (b = m) : (a = m)
    end
    (a + b) / 2
end

# ── ODE solver: RK4 ──────────────────────────────────────────────────────────

function rk4(f, t0, y0, t_end, h)
    ts = Float64[t0]
    ys = [y0]
    t, y = t0, y0
    while t < t_end
        k1 = f(t,        y)
        k2 = f(t + h/2,  y + h/2 * k1)
        k3 = f(t + h/2,  y + h/2 * k2)
        k4 = f(t + h,    y + h   * k3)
        y  = y + h/6 * (k1 + 2k2 + 2k3 + k4)
        t  = t + h
        push!(ts, t); push!(ys, y)
    end
    ts, ys
end

# ── Demo ──────────────────────────────────────────────────────────────────────

root = newton_raphson(x -> x^3 - 2, x -> 3x^2, 1.5)
println("∛2 ≈ $root  (error: $(abs(root - 2^(1/3))))")

# dy/dt = -y  →  y(t) = exp(-t)
ts, ys = rk4((t, y) -> -y, 0.0, 1.0, 2.0, 0.01)
println("y(2) ≈ $(last(ys))  (exact: $(exp(-2.0)))")

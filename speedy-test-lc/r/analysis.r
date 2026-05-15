# Statistical analysis: compare two groups with bootstrap confidence intervals.

set.seed(42)

# Simulate two groups of measurements
group_a <- rnorm(60, mean = 100, sd = 15)
group_b <- rnorm(60, mean = 108, sd = 18)

# ── Descriptive statistics ────────────────────────────────────────────────────
describe <- function(x, label) {
  cat(sprintf(
    "%s  n=%d  mean=%.2f  sd=%.2f  median=%.2f  IQR=[%.2f, %.2f]\n",
    label, length(x), mean(x), sd(x), median(x), quantile(x, .25), quantile(x, .75)
  ))
}

describe(group_a, "Group A")
describe(group_b, "Group B")

# ── Bootstrap CI for difference of means ─────────────────────────────────────
bootstrap_diff <- function(a, b, n_boot = 5000, alpha = 0.05) {
  diffs <- replicate(n_boot, {
    mean(sample(a, replace = TRUE)) - mean(sample(b, replace = TRUE))
  })
  ci <- quantile(diffs, c(alpha / 2, 1 - alpha / 2))
  list(
    observed = mean(a) - mean(b),
    ci_lower = ci[[1]],
    ci_upper = ci[[2]]
  )
}

result <- bootstrap_diff(group_a, group_b)
cat(sprintf(
  "\nDifference A-B: %.2f  95%% CI [%.2f, %.2f]\n",
  result$observed, result$ci_lower, result$ci_upper
))

# ── Welch t-test ──────────────────────────────────────────────────────────────
t_result <- t.test(group_a, group_b)
cat(sprintf("Welch t-test  t=%.3f  df=%.1f  p=%.4f\n",
  t_result$statistic, t_result$parameter, t_result$p.value))

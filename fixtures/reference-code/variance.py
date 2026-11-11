def sample_variance(values):
    count, mean, second = 0, 0.0, 0.0
    for value in values:
        count += 1
        delta = value - mean
        mean += delta / count
        second += delta * (value - mean)
    if count < 2:
        raise ValueError("at least two samples required")
    return second / (count - 1)

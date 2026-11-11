def greatest_common_divisor(a, b):
    a, b = abs(a), abs(b)
    while b:
        a, b = b, a % b
    return a


def least_common_multiple(a, b):
    if a == 0 or b == 0:
        return 0
    return abs(a // greatest_common_divisor(a, b) * b)

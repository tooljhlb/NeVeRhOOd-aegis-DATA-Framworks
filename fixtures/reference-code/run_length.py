def encode_runs(values):
    output = []
    for value in values:
        if output and output[-1][0] == value:
            output[-1][1] += 1
        else:
            output.append([value, 1])
    return output


def decode_runs(runs):
    return [value for value, count in runs for _ in range(count)]

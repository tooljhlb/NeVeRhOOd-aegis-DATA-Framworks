def matrix_product(a, b):
    if not a or not b or any(len(row) != len(b) for row in a):
        raise ValueError("incompatible matrix shapes")
    columns = len(b[0])
    if any(len(row) != columns for row in b):
        raise ValueError("ragged right matrix")
    return [[sum(row[k] * b[k][j] for k in range(len(b))) for j in range(columns)] for row in a]

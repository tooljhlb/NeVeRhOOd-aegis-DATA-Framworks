def topological_order(graph):
    active, done, result = set(), set(), []

    def visit(node):
        if node in active:
            raise ValueError("cycle")
        if node in done:
            return
        active.add(node)
        for dependency in graph.get(node, []):
            visit(dependency)
        active.remove(node)
        done.add(node)
        result.append(node)

    for node in graph:
        visit(node)
    return result

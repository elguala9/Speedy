#include "heap.hpp"
#include <vector>
#include <limits>
#include <utility>

using Graph = std::vector<std::vector<std::pair<int, int>>>; // adj[u] = {(v, weight)}

std::vector<int> dijkstra(const Graph &graph, int source) {
    int n = static_cast<int>(graph.size());
    std::vector<int> dist(n, std::numeric_limits<int>::max());
    dist[source] = 0;

    // min-heap on (distance, node)
    using P = std::pair<int,int>;
    BinaryHeap<P, std::greater<P>> pq;
    pq.push({0, source});

    while (!pq.empty()) {
        auto [d, u] = pq.top(); pq.pop();
        if (d > dist[u]) continue;
        for (auto [v, w] : graph[u]) {
            if (dist[u] + w < dist[v]) {
                dist[v] = dist[u] + w;
                pq.push({dist[v], v});
            }
        }
    }
    return dist;
}

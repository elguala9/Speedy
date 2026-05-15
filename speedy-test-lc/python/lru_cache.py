"""LRU cache backed by a doubly-linked list + hash map."""
from __future__ import annotations
from typing import Generic, Hashable, Optional, TypeVar

K = TypeVar("K", bound=Hashable)
V = TypeVar("V")


class _Node(Generic[K, V]):
    __slots__ = ("key", "value", "prev", "next")

    def __init__(self, key: K, value: V) -> None:
        self.key   = key
        self.value = value
        self.prev: Optional[_Node] = None
        self.next: Optional[_Node] = None


class LRUCache(Generic[K, V]):
    def __init__(self, capacity: int) -> None:
        if capacity < 1:
            raise ValueError("capacity must be >= 1")
        self._cap   = capacity
        self._map: dict[K, _Node[K, V]] = {}
        # sentinel head/tail so we never deal with None neighbours
        self._head  = _Node(None, None)   # type: ignore[arg-type]
        self._tail  = _Node(None, None)   # type: ignore[arg-type]
        self._head.next = self._tail
        self._tail.prev = self._head

    # ── public API ────────────────────────────────────────────────────────

    def get(self, key: K) -> Optional[V]:
        node = self._map.get(key)
        if node is None:
            return None
        self._move_to_front(node)
        return node.value

    def put(self, key: K, value: V) -> None:
        if key in self._map:
            node = self._map[key]
            node.value = value
            self._move_to_front(node)
        else:
            node = _Node(key, value)
            self._map[key] = node
            self._insert_front(node)
            if len(self._map) > self._cap:
                self._evict_lru()

    def __len__(self) -> int:
        return len(self._map)

    # ── internals ────────────────────────────────────────────────────────

    def _insert_front(self, node: _Node) -> None:
        node.prev = self._head
        node.next = self._head.next
        self._head.next.prev = node
        self._head.next      = node

    def _remove(self, node: _Node) -> None:
        node.prev.next = node.next
        node.next.prev = node.prev

    def _move_to_front(self, node: _Node) -> None:
        self._remove(node)
        self._insert_front(node)

    def _evict_lru(self) -> None:
        lru = self._tail.prev
        self._remove(lru)
        del self._map[lru.key]

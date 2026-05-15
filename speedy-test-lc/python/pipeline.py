"""Simple data-processing pipeline with composable stages."""
from __future__ import annotations
from dataclasses import dataclass, field
from typing import Callable, Generic, Iterable, Iterator, TypeVar

T = TypeVar("T")
U = TypeVar("U")


@dataclass
class Stage(Generic[T, U]):
    name: str
    fn: Callable[[T], U]

    def __call__(self, value: T) -> U:
        return self.fn(value)


@dataclass
class Pipeline(Generic[T]):
    stages: list[Stage] = field(default_factory=list)

    def pipe(self, name: str, fn: Callable) -> "Pipeline":
        self.stages.append(Stage(name=name, fn=fn))
        return self

    def run(self, source: Iterable[T]) -> Iterator:
        for item in source:
            result = item
            for stage in self.stages:
                result = stage(result)
            yield result

    def run_list(self, source: Iterable[T]) -> list:
        return list(self.run(source))


def words_pipeline() -> Pipeline:
    return (
        Pipeline()
        .pipe("strip",    str.strip)
        .pipe("lower",    str.lower)
        .pipe("tokenise", str.split)
        .pipe("filter",   lambda tokens: [t for t in tokens if len(t) > 2])
    )


if __name__ == "__main__":
    lines = [
        "  The quick brown fox  ",
        "Jumps Over the Lazy Dog",
        "a to be or not to be",
    ]
    pl = words_pipeline()
    for tokens in pl.run(lines):
        print(tokens)

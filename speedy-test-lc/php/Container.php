<?php

declare(strict_types=1);

/**
 * Minimal PSR-11-style dependency-injection container.
 */
class Container
{
    /** @var array<string, callable> */
    private array $bindings = [];
    /** @var array<string, mixed> */
    private array $singletons = [];
    /** @var array<string, bool> */
    private array $shared = [];

    public function bind(string $abstract, callable $factory, bool $singleton = false): void
    {
        $this->bindings[$abstract] = $factory;
        $this->shared[$abstract]   = $singleton;
        unset($this->singletons[$abstract]);
    }

    public function singleton(string $abstract, callable $factory): void
    {
        $this->bind($abstract, $factory, singleton: true);
    }

    public function make(string $abstract): mixed
    {
        if (isset($this->singletons[$abstract])) {
            return $this->singletons[$abstract];
        }

        if (!isset($this->bindings[$abstract])) {
            return $this->autowire($abstract);
        }

        $instance = ($this->bindings[$abstract])($this);

        if ($this->shared[$abstract]) {
            $this->singletons[$abstract] = $instance;
        }

        return $instance;
    }

    private function autowire(string $class): object
    {
        $ref    = new ReflectionClass($class);
        $ctor   = $ref->getConstructor();

        if ($ctor === null) {
            return $ref->newInstance();
        }

        $args = array_map(
            fn(ReflectionParameter $p): mixed => $this->make(
                $p->getType()?->getName()
                ?? throw new RuntimeException("Cannot autowire parameter \${$p->getName()} of {$class}")
            ),
            $ctor->getParameters()
        );

        return $ref->newInstanceArgs($args);
    }
}

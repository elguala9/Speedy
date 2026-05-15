/** Railway-oriented Result<T, E> monad. */

export type Result<T, E = Error> = Ok<T, E> | Err<T, E>;

export class Ok<T, E> {
  readonly _tag = "Ok" as const;
  constructor(readonly value: T) {}

  isOk(): this is Ok<T, E>   { return true; }
  isErr(): this is Err<T, E> { return false; }

  map<U>(fn: (value: T) => U): Result<U, E>  { return ok(fn(this.value)); }
  flatMap<U>(fn: (value: T) => Result<U, E>) { return fn(this.value); }
  mapErr<F>(_fn: (err: E) => F): Result<T, F>  { return ok(this.value); }
  unwrap(): T    { return this.value; }
  unwrapOr(_fallback: T): T { return this.value; }
}

export class Err<T, E> {
  readonly _tag = "Err" as const;
  constructor(readonly error: E) {}

  isOk(): this is Ok<T, E>   { return false; }
  isErr(): this is Err<T, E> { return true; }

  map<U>(_fn: (value: T) => U): Result<U, E>    { return err(this.error); }
  flatMap<U>(_fn: (value: T) => Result<U, E>)   { return err<U, E>(this.error); }
  mapErr<F>(fn: (err: E) => F): Result<T, F>    { return err(fn(this.error)); }
  unwrap(): never { throw this.error; }
  unwrapOr(fallback: T): T { return fallback; }
}

export const ok  = <T, E = never>(value: T): Result<T, E> => new Ok(value);
export const err = <T = never, E = Error>(error: E): Result<T, E> => new Err(error);

export function tryCatch<T>(fn: () => T): Result<T, Error> {
  try   { return ok(fn()); }
  catch (e) { return err(e instanceof Error ? e : new Error(String(e))); }
}

import 'dart:async';

/// Immutable state container with subscription support (mini-Redux).
class Store<S> {
  S _state;
  final S Function(S state, Object action) _reducer;
  final _controller = StreamController<S>.broadcast();

  Store(this._state, this._reducer);

  S get state => _state;

  Stream<S> get stream => _controller.stream;

  void dispatch(Object action) {
    _state = _reducer(_state, action);
    _controller.add(_state);
  }

  StreamSubscription<S> subscribe(void Function(S state) listener) =>
      stream.listen(listener);

  void dispose() => _controller.close();
}

// ── Example: counter ────────────────────────────────────────────────────────

sealed class CounterAction {}
class Increment extends CounterAction { final int by; Increment([this.by = 1]); }
class Decrement extends CounterAction { final int by; Decrement([this.by = 1]); }
class Reset     extends CounterAction {}

int counterReducer(int state, Object action) => switch (action) {
  Increment(:final by) => state + by,
  Decrement(:final by) => state - by,
  Reset()              => 0,
  _                    => state,
};

void main() {
  final store = Store<int>(0, counterReducer);
  store.subscribe((s) => print('state: $s'));

  store.dispatch(Increment(5));
  store.dispatch(Increment());
  store.dispatch(Decrement(2));
  store.dispatch(Reset());
  store.dispose();
}

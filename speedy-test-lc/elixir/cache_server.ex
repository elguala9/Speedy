defmodule SpeedyTest.CacheServer do
  @moduledoc """
  A GenServer-backed TTL cache.
  Each entry expires after `ttl_ms` milliseconds.
  """
  use GenServer

  @type key   :: any()
  @type value :: any()

  # ── Public API ─────────────────────────────────────────────────────────

  def start_link(opts \\ []) do
    name = Keyword.get(opts, :name, __MODULE__)
    GenServer.start_link(__MODULE__, %{}, name: name)
  end

  @spec put(GenServer.server(), key(), value(), pos_integer()) :: :ok
  def put(server, key, value, ttl_ms) do
    GenServer.call(server, {:put, key, value, ttl_ms})
  end

  @spec get(GenServer.server(), key()) :: {:ok, value()} | :miss
  def get(server, key) do
    GenServer.call(server, {:get, key})
  end

  @spec delete(GenServer.server(), key()) :: :ok
  def delete(server, key) do
    GenServer.call(server, {:delete, key})
  end

  # ── Callbacks ──────────────────────────────────────────────────────────

  @impl true
  def init(state), do: {:ok, state}

  @impl true
  def handle_call({:put, key, value, ttl_ms}, _from, state) do
    timer = Process.send_after(self(), {:expire, key}, ttl_ms)
    {:reply, :ok, Map.put(state, key, {value, timer})}
  end

  def handle_call({:get, key}, _from, state) do
    reply =
      case Map.fetch(state, key) do
        {:ok, {value, _timer}} -> {:ok, value}
        :error                 -> :miss
      end
    {:reply, reply, state}
  end

  def handle_call({:delete, key}, _from, state) do
    state =
      case Map.pop(state, key) do
        {{_v, timer}, new_state} ->
          Process.cancel_timer(timer)
          new_state
        {nil, state} ->
          state
      end
    {:reply, :ok, state}
  end

  @impl true
  def handle_info({:expire, key}, state) do
    {:noreply, Map.delete(state, key)}
  end
end

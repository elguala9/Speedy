%% Erlang gen_server: a named counter with increment / decrement / reset.
-module(counter).
-behaviour(gen_server).

-export([start_link/1, increment/1, increment/2, decrement/1, value/1, reset/1]).
-export([init/1, handle_call/3, handle_cast/2, handle_info/2, terminate/2]).

%% ── Public API ────────────────────────────────────────────────────────

start_link(Name) ->
    gen_server:start_link({local, Name}, ?MODULE, 0, []).

increment(Name) -> increment(Name, 1).
increment(Name, By) -> gen_server:call(Name, {increment, By}).

decrement(Name) -> gen_server:call(Name, {increment, -1}).

value(Name) -> gen_server:call(Name, value).

reset(Name) -> gen_server:cast(Name, reset).

%% ── Callbacks ────────────────────────────────────────────────────────

init(Initial) -> {ok, Initial}.

handle_call({increment, By}, _From, Count) ->
    New = Count + By,
    {reply, New, New};
handle_call(value, _From, Count) ->
    {reply, Count, Count}.

handle_cast(reset, _Count) ->
    {noreply, 0}.

handle_info(_Msg, State) -> {noreply, State}.

terminate(_Reason, _State) -> ok.

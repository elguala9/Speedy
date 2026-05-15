--- Minimal finite-state machine library.

local StateMachine = {}
StateMachine.__index = StateMachine

function StateMachine.new(initial)
    return setmetatable({
        state       = initial,
        transitions = {},   -- [from][event] = { to, action }
        listeners   = {},   -- [event] = [fn, ...]
    }, StateMachine)
end

function StateMachine:add_transition(from, event, to, action)
    self.transitions[from] = self.transitions[from] or {}
    self.transitions[from][event] = { to = to, action = action }
    return self
end

function StateMachine:on(event, fn)
    self.listeners[event] = self.listeners[event] or {}
    table.insert(self.listeners[event], fn)
    return self
end

function StateMachine:send(event, ...)
    local from_transitions = self.transitions[self.state]
    if not from_transitions then
        error(("No transitions from state '%s'"):format(self.state))
    end
    local t = from_transitions[event]
    if not t then
        error(("No transition for event '%s' in state '%s'"):format(event, self.state))
    end

    if t.action then t.action(self.state, t.to, ...) end

    local prev  = self.state
    self.state  = t.to

    for _, fn in ipairs(self.listeners[event] or {}) do
        fn(prev, t.to, ...)
    end
end

-- Example: traffic light
local light = StateMachine.new("red")
    :add_transition("red",    "go",   "green")
    :add_transition("green",  "slow", "yellow")
    :add_transition("yellow", "stop", "red",   function(f, t) print(f.." -> "..t) end)

light:send("go")
light:send("slow")
light:send("stop")
print("Final state:", light.state)

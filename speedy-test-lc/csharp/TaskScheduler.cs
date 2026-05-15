using System;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;

namespace SpeedyTest;

public sealed class ScheduledTask
{
    public string   Name      { get; init; } = string.Empty;
    public TimeSpan Interval  { get; init; }
    public Func<CancellationToken, Task> Action { get; init; } = _ => Task.CompletedTask;
}

public sealed class TaskScheduler : IAsyncDisposable
{
    private readonly List<(ScheduledTask Task, Timer Timer)> _registrations = [];
    private readonly CancellationTokenSource _cts = new();

    public void Register(ScheduledTask task)
    {
        var timer = new Timer(
            async _ =>
            {
                try   { await task.Action(_cts.Token); }
                catch (OperationCanceledException) { }
                catch (Exception ex) { Console.Error.WriteLine($"[{task.Name}] {ex.Message}"); }
            },
            state: null,
            dueTime:  task.Interval,
            period:   task.Interval);

        _registrations.Add((task, timer));
    }

    public async ValueTask DisposeAsync()
    {
        await _cts.CancelAsync();
        foreach (var (_, timer) in _registrations)
            await timer.DisposeAsync();
        _cts.Dispose();
    }
}

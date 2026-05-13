// Oracle host. Reflectively loads the shipping sts2.dll and exposes its
// functions over stdio JSON-RPC. Each request is a single JSON line with
// `{ "method": ..., "params": {...} }` and the response is a single JSON line
// with either `{ "result": ... }` or `{ "error": "..." }`.
//
// State (Rng instances etc.) is held by integer handles. Tests `rng_new` to
// get a handle, then call other rng_* methods passing `handle`.

using System.Collections.Generic;
using System.Reflection;
using System.Runtime.Loader;
using System.Text.Json.Nodes;

namespace Sts2Sim.OracleHost;

internal static class Program
{
    private const string DefaultGameDir =
        @"G:\SteamLibrary\steamapps\common\Slay the Spire 2\data_sts2_windows_x86_64";

    private static int Main(string[] args)
    {
        var gameDir = Environment.GetEnvironmentVariable("STS2_GAME_DIR") ?? DefaultGameDir;
        if (!Directory.Exists(gameDir))
        {
            Console.Error.WriteLine($"oracle-host: game directory not found: {gameDir}");
            return 2;
        }

        var dllPath = Path.Combine(gameDir, "sts2.dll");
        if (!File.Exists(dllPath))
        {
            Console.Error.WriteLine($"oracle-host: sts2.dll not found at {dllPath}");
            return 2;
        }

        Dispatcher dispatcher;
        try
        {
            dispatcher = Dispatcher.Create(gameDir, dllPath);
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"oracle-host: failed to initialize dispatcher: {ex}");
            return 3;
        }

        string? line;
        while ((line = Console.In.ReadLine()) is not null)
        {
            JsonObject response;
            try
            {
                var req = JsonNode.Parse(line)?.AsObject()
                    ?? throw new InvalidOperationException("request was not a JSON object");
                var method = req["method"]?.GetValue<string>() ?? "";
                var p = req["params"] as JsonObject ?? new JsonObject();
                response = dispatcher.Dispatch(method, p);
            }
            catch (TargetInvocationException ex) when (ex.InnerException is not null)
            {
                response = new JsonObject { ["error"] = ex.InnerException.Message };
            }
            catch (Exception ex)
            {
                response = new JsonObject { ["error"] = ex.Message };
            }

            Console.Out.WriteLine(response.ToJsonString());
            Console.Out.Flush();
        }
        return 0;
    }
}

// Custom AssemblyLoadContext that resolves dependencies (GodotSharp, 0Harmony,
// etc.) from the game's data directory rather than the oracle-host's own
// runtime. Without this, sts2.dll fails to load because its dependencies aren't
// alongside our exe.
internal sealed class GameLoadContext : AssemblyLoadContext
{
    private readonly string _gameDir;

    public GameLoadContext(string gameDir) : base("sts2-game", isCollectible: false)
    {
        _gameDir = gameDir;
    }

    protected override Assembly? Load(AssemblyName assemblyName)
    {
        var candidate = Path.Combine(_gameDir, assemblyName.Name + ".dll");
        return File.Exists(candidate) ? LoadFromAssemblyPath(candidate) : null;
    }
}

internal sealed class Dispatcher
{
    private readonly Dictionary<int, object> _instances = new();
    private int _nextHandle = 1;

    private readonly Type _rngType;
    private readonly ConstructorInfo _rngCtor;
    private readonly MethodInfo _nextIntSingle;
    private readonly MethodInfo _nextIntRange;
    private readonly MethodInfo _nextBool;
    private readonly MethodInfo _fastForward;
    private readonly MethodInfo _shuffleGeneric;
    private readonly PropertyInfo _counterProp;
    private readonly PropertyInfo _seedProp;

    private Dispatcher(
        Type rngType,
        ConstructorInfo ctor,
        MethodInfo nextIntSingle,
        MethodInfo nextIntRange,
        MethodInfo nextBool,
        MethodInfo fastForward,
        MethodInfo shuffleGeneric,
        PropertyInfo counterProp,
        PropertyInfo seedProp)
    {
        _rngType = rngType;
        _rngCtor = ctor;
        _nextIntSingle = nextIntSingle;
        _nextIntRange = nextIntRange;
        _nextBool = nextBool;
        _fastForward = fastForward;
        _shuffleGeneric = shuffleGeneric;
        _counterProp = counterProp;
        _seedProp = seedProp;
    }

    public static Dispatcher Create(string gameDir, string dllPath)
    {
        var alc = new GameLoadContext(gameDir);
        var asm = alc.LoadFromAssemblyPath(dllPath);
        var rngType = asm.GetType("MegaCrit.Sts2.Core.Random.Rng", throwOnError: true)!;

        var ctor = rngType.GetConstructor(new[] { typeof(uint), typeof(int) })
            ?? throw new InvalidOperationException("Rng(uint, int) ctor not found");

        var methods = rngType.GetMethods();
        var nextIntSingle = methods.First(m =>
            m.Name == "NextInt"
            && m.GetParameters().Length == 1
            && m.GetParameters()[0].ParameterType == typeof(int));
        var nextIntRange = methods.First(m =>
            m.Name == "NextInt" && m.GetParameters().Length == 2);
        var nextBool = methods.First(m =>
            m.Name == "NextBool" && m.GetParameters().Length == 0);
        var fastForward = methods.First(m => m.Name == "FastForwardCounter");
        var shuffleGen = methods.First(m => m.Name == "Shuffle" && m.IsGenericMethod);

        var counter = rngType.GetProperty("Counter")
            ?? throw new InvalidOperationException("Counter property not found");
        var seed = rngType.GetProperty("Seed")
            ?? throw new InvalidOperationException("Seed property not found");

        return new Dispatcher(rngType, ctor, nextIntSingle, nextIntRange,
            nextBool, fastForward, shuffleGen, counter, seed);
    }

    public JsonObject Dispatch(string method, JsonObject p)
    {
        switch (method)
        {
            case "ping":
                return Ok(JsonValue.Create("pong"));

            case "rng_new":
            {
                var seed = (uint)p["seed"]!.GetValue<long>();
                var counter = p["counter"]?.GetValue<int>() ?? 0;
                var inst = _rngCtor.Invoke(new object[] { seed, counter });
                var handle = _nextHandle++;
                _instances[handle] = inst!;
                return Ok(JsonValue.Create(handle));
            }

            case "rng_next_int":
            {
                var inst = GetInstance(p);
                var max = p["max_exclusive"]!.GetValue<int>();
                var v = (int)_nextIntSingle.Invoke(inst, new object[] { max })!;
                return Ok(JsonValue.Create(v));
            }

            case "rng_next_int_range":
            {
                var inst = GetInstance(p);
                var min = p["min_inclusive"]!.GetValue<int>();
                var max = p["max_exclusive"]!.GetValue<int>();
                var v = (int)_nextIntRange.Invoke(inst, new object[] { min, max })!;
                return Ok(JsonValue.Create(v));
            }

            case "rng_next_bool":
            {
                var inst = GetInstance(p);
                var v = (bool)_nextBool.Invoke(inst, Array.Empty<object>())!;
                return Ok(JsonValue.Create(v));
            }

            case "rng_fast_forward":
            {
                var inst = GetInstance(p);
                var target = p["target_count"]!.GetValue<int>();
                _fastForward.Invoke(inst, new object[] { target });
                return Ok(JsonValue.Create(true));
            }

            case "rng_shuffle":
            {
                var inst = GetInstance(p);
                var arr = p["list"]!.AsArray()
                    .Select(n => n!.GetValue<int>())
                    .ToList();
                var shuffleInt = _shuffleGeneric.MakeGenericMethod(typeof(int));
                shuffleInt.Invoke(inst, new object[] { arr });
                var result = new JsonArray();
                foreach (var v in arr) result.Add(v);
                return Ok(result);
            }

            case "rng_counter":
            {
                var inst = GetInstance(p);
                return Ok(JsonValue.Create((int)_counterProp.GetValue(inst)!));
            }

            case "rng_seed":
            {
                var inst = GetInstance(p);
                return Ok(JsonValue.Create((long)(uint)_seedProp.GetValue(inst)!));
            }

            case "rng_dispose":
            {
                var handle = p["handle"]!.GetValue<int>();
                _instances.Remove(handle);
                return Ok(JsonValue.Create(true));
            }

            default:
                return new JsonObject { ["error"] = $"unknown method: {method}" };
        }
    }

    private object GetInstance(JsonObject p)
    {
        var handle = p["handle"]!.GetValue<int>();
        if (!_instances.TryGetValue(handle, out var inst))
            throw new InvalidOperationException($"no rng instance for handle {handle}");
        return inst;
    }

    private static JsonObject Ok(JsonNode? result) => new() { ["result"] = result };
}

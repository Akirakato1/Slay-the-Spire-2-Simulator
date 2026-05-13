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
    private readonly ConstructorInfo _rngCtorNamed;
    private readonly MethodInfo _stringHelperHash;
    private readonly MethodInfo _stringHelperSnake;
    private readonly MethodInfo _listExtStableShuffle;
    private readonly MethodInfo _nextIntSingle;
    private readonly MethodInfo _nextIntRange;
    private readonly MethodInfo _nextBool;
    private readonly MethodInfo _nextDoubleSingle;
    private readonly MethodInfo _nextDoubleRange;
    private readonly MethodInfo _nextFloatSingle;
    private readonly MethodInfo _nextFloatRange;
    private readonly MethodInfo _nextUIntSingle;
    private readonly MethodInfo _nextUIntRange;
    private readonly MethodInfo _nextGaussianDouble;
    private readonly MethodInfo _nextGaussianFloat;
    private readonly MethodInfo _nextGaussianInt;
    private readonly MethodInfo _fastForward;
    private readonly MethodInfo _shuffleGeneric;
    private readonly MethodInfo _nextItemGeneric;
    private readonly MethodInfo _weightedNextItemGeneric;
    private readonly PropertyInfo _counterProp;
    private readonly PropertyInfo _seedProp;

    private Dispatcher(
        Type rngType,
        ConstructorInfo ctor,
        ConstructorInfo ctorNamed,
        MethodInfo stringHelperHash,
        MethodInfo stringHelperSnake,
        MethodInfo listExtStableShuffle,
        MethodInfo nextIntSingle,
        MethodInfo nextIntRange,
        MethodInfo nextBool,
        MethodInfo nextDoubleSingle,
        MethodInfo nextDoubleRange,
        MethodInfo nextFloatSingle,
        MethodInfo nextFloatRange,
        MethodInfo nextUIntSingle,
        MethodInfo nextUIntRange,
        MethodInfo nextGaussianDouble,
        MethodInfo nextGaussianFloat,
        MethodInfo nextGaussianInt,
        MethodInfo fastForward,
        MethodInfo shuffleGeneric,
        MethodInfo nextItemGeneric,
        MethodInfo weightedNextItemGeneric,
        PropertyInfo counterProp,
        PropertyInfo seedProp)
    {
        _rngType = rngType;
        _rngCtor = ctor;
        _rngCtorNamed = ctorNamed;
        _stringHelperHash = stringHelperHash;
        _stringHelperSnake = stringHelperSnake;
        _listExtStableShuffle = listExtStableShuffle;
        _nextIntSingle = nextIntSingle;
        _nextIntRange = nextIntRange;
        _nextBool = nextBool;
        _nextDoubleSingle = nextDoubleSingle;
        _nextDoubleRange = nextDoubleRange;
        _nextFloatSingle = nextFloatSingle;
        _nextFloatRange = nextFloatRange;
        _nextUIntSingle = nextUIntSingle;
        _nextUIntRange = nextUIntRange;
        _nextGaussianDouble = nextGaussianDouble;
        _nextGaussianFloat = nextGaussianFloat;
        _nextGaussianInt = nextGaussianInt;
        _fastForward = fastForward;
        _shuffleGeneric = shuffleGeneric;
        _nextItemGeneric = nextItemGeneric;
        _weightedNextItemGeneric = weightedNextItemGeneric;
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
        var ctorNamed = rngType.GetConstructor(new[] { typeof(uint), typeof(string) })
            ?? throw new InvalidOperationException("Rng(uint, string) ctor not found");

        var stringHelperType = asm.GetType("MegaCrit.Sts2.Core.Helpers.StringHelper",
            throwOnError: true)!;
        var stringHelperHash = stringHelperType.GetMethod("GetDeterministicHashCode",
            BindingFlags.Public | BindingFlags.Static)
            ?? throw new InvalidOperationException("StringHelper.GetDeterministicHashCode not found");
        var stringHelperSnake = stringHelperType.GetMethod("SnakeCase",
            BindingFlags.Public | BindingFlags.Static)
            ?? throw new InvalidOperationException("StringHelper.SnakeCase not found");

        var listExtType = asm.GetType("MegaCrit.Sts2.Core.Extensions.ListExtensions",
            throwOnError: true)!;
        var listExtStableShuffle = listExtType.GetMethods()
            .First(m => m.Name == "StableShuffle" && m.IsGenericMethod);

        var methods = rngType.GetMethods();
        MethodInfo Find(string name, int arity, Type? firstParam = null) =>
            methods.First(m => m.Name == name
                && m.GetParameters().Length == arity
                && !m.IsGenericMethod
                && (firstParam is null || m.GetParameters()[0].ParameterType == firstParam));

        var nextIntSingle = Find("NextInt", 1, typeof(int));
        var nextIntRange = Find("NextInt", 2, typeof(int));
        var nextBool = Find("NextBool", 0);
        var nextDoubleSingle = Find("NextDouble", 0);
        var nextDoubleRange = Find("NextDouble", 2, typeof(double));
        var nextFloatSingle = Find("NextFloat", 1, typeof(float));
        var nextFloatRange = Find("NextFloat", 2, typeof(float));
        var nextUIntSingle = Find("NextUnsignedInt", 1, typeof(uint));
        var nextUIntRange = Find("NextUnsignedInt", 2, typeof(uint));
        var nextGaussianDouble = Find("NextGaussianDouble", 4, typeof(double));
        var nextGaussianFloat = Find("NextGaussianFloat", 4, typeof(float));
        var nextGaussianInt = Find("NextGaussianInt", 4, typeof(int));
        var fastForward = methods.First(m => m.Name == "FastForwardCounter");
        var shuffleGen = methods.First(m => m.Name == "Shuffle" && m.IsGenericMethod);
        var nextItemGen = methods.First(m =>
            m.Name == "NextItem" && m.IsGenericMethod && m.GetParameters().Length == 1);
        // The instance WeightedNextItem (NOT the static one): two params
        // (IEnumerable<T>, Func<T, float>) and is_generic.
        var weightedNextItemGen = methods.First(m =>
            m.Name == "WeightedNextItem"
            && m.IsGenericMethod
            && m.GetParameters().Length == 2
            && !m.IsStatic);

        var counter = rngType.GetProperty("Counter")
            ?? throw new InvalidOperationException("Counter property not found");
        var seed = rngType.GetProperty("Seed")
            ?? throw new InvalidOperationException("Seed property not found");

        return new Dispatcher(rngType, ctor, ctorNamed,
            stringHelperHash, stringHelperSnake, listExtStableShuffle,
            nextIntSingle, nextIntRange,
            nextBool, nextDoubleSingle, nextDoubleRange,
            nextFloatSingle, nextFloatRange, nextUIntSingle, nextUIntRange,
            nextGaussianDouble, nextGaussianFloat, nextGaussianInt,
            fastForward, shuffleGen, nextItemGen, weightedNextItemGen,
            counter, seed);
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

            case "rng_new_named":
            {
                var seed = (uint)p["seed"]!.GetValue<long>();
                var name = p["name"]!.GetValue<string>();
                var inst = _rngCtorNamed.Invoke(new object[] { seed, name });
                var handle = _nextHandle++;
                _instances[handle] = inst!;
                return Ok(JsonValue.Create(handle));
            }

            case "hash_string":
            {
                var s = p["str"]!.GetValue<string>();
                var v = (int)_stringHelperHash.Invoke(null, new object[] { s })!;
                return Ok(JsonValue.Create(v));
            }

            case "snake_case":
            {
                var s = p["str"]!.GetValue<string>();
                var v = (string)_stringHelperSnake.Invoke(null, new object[] { s })!;
                return Ok(JsonValue.Create(v));
            }

            case "stable_shuffle":
            {
                var inst = GetInstance(p);
                var arr = p["list"]!.AsArray()
                    .Select(n => n!.GetValue<int>()).ToList();
                var ssInt = _listExtStableShuffle.MakeGenericMethod(typeof(int));
                ssInt.Invoke(null, new object[] { arr, inst });
                var result = new JsonArray();
                foreach (var v in arr) result.Add(v);
                return Ok(result);
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

            case "rng_next_double":
            {
                var inst = GetInstance(p);
                var v = (double)_nextDoubleSingle.Invoke(inst, Array.Empty<object>())!;
                // Serialize as the bit pattern of the f64 so JSON round-trips
                // are exact, immune to formatter precision quirks.
                return Ok(JsonValue.Create(BitConverter.DoubleToInt64Bits(v)));
            }

            case "rng_next_double_range":
            {
                var inst = GetInstance(p);
                var min = BitConverter.Int64BitsToDouble(p["min_bits"]!.GetValue<long>());
                var max = BitConverter.Int64BitsToDouble(p["max_bits"]!.GetValue<long>());
                var v = (double)_nextDoubleRange.Invoke(inst, new object[] { min, max })!;
                return Ok(JsonValue.Create(BitConverter.DoubleToInt64Bits(v)));
            }

            case "rng_next_float":
            {
                var inst = GetInstance(p);
                var max = BitConverter.Int32BitsToSingle(p["max_bits"]!.GetValue<int>());
                var v = (float)_nextFloatSingle.Invoke(inst, new object[] { max })!;
                return Ok(JsonValue.Create(BitConverter.SingleToInt32Bits(v)));
            }

            case "rng_next_float_range":
            {
                var inst = GetInstance(p);
                var min = BitConverter.Int32BitsToSingle(p["min_bits"]!.GetValue<int>());
                var max = BitConverter.Int32BitsToSingle(p["max_bits"]!.GetValue<int>());
                var v = (float)_nextFloatRange.Invoke(inst, new object[] { min, max })!;
                return Ok(JsonValue.Create(BitConverter.SingleToInt32Bits(v)));
            }

            case "rng_next_uint":
            {
                var inst = GetInstance(p);
                var max = (uint)p["max_exclusive"]!.GetValue<long>();
                var v = (uint)_nextUIntSingle.Invoke(inst, new object[] { max })!;
                return Ok(JsonValue.Create((long)v));
            }

            case "rng_next_uint_range":
            {
                var inst = GetInstance(p);
                var min = (uint)p["min_inclusive"]!.GetValue<long>();
                var max = (uint)p["max_exclusive"]!.GetValue<long>();
                var v = (uint)_nextUIntRange.Invoke(inst, new object[] { min, max })!;
                return Ok(JsonValue.Create((long)v));
            }

            case "rng_next_gaussian_double":
            {
                var inst = GetInstance(p);
                var mean = BitConverter.Int64BitsToDouble(p["mean_bits"]!.GetValue<long>());
                var std = BitConverter.Int64BitsToDouble(p["std_dev_bits"]!.GetValue<long>());
                var min = BitConverter.Int64BitsToDouble(p["min_bits"]!.GetValue<long>());
                var max = BitConverter.Int64BitsToDouble(p["max_bits"]!.GetValue<long>());
                var v = (double)_nextGaussianDouble.Invoke(inst,
                    new object[] { mean, std, min, max })!;
                return Ok(JsonValue.Create(BitConverter.DoubleToInt64Bits(v)));
            }

            case "rng_next_gaussian_float":
            {
                var inst = GetInstance(p);
                var mean = BitConverter.Int32BitsToSingle(p["mean_bits"]!.GetValue<int>());
                var std = BitConverter.Int32BitsToSingle(p["std_dev_bits"]!.GetValue<int>());
                var min = BitConverter.Int32BitsToSingle(p["min_bits"]!.GetValue<int>());
                var max = BitConverter.Int32BitsToSingle(p["max_bits"]!.GetValue<int>());
                var v = (float)_nextGaussianFloat.Invoke(inst,
                    new object[] { mean, std, min, max })!;
                return Ok(JsonValue.Create(BitConverter.SingleToInt32Bits(v)));
            }

            case "rng_next_gaussian_int":
            {
                var inst = GetInstance(p);
                var mean = p["mean"]!.GetValue<int>();
                var std = p["std_dev"]!.GetValue<int>();
                var min = p["min"]!.GetValue<int>();
                var max = p["max"]!.GetValue<int>();
                var v = (int)_nextGaussianInt.Invoke(inst,
                    new object[] { mean, std, min, max })!;
                return Ok(JsonValue.Create(v));
            }

            case "rng_fast_forward":
            {
                var inst = GetInstance(p);
                var target = p["target_count"]!.GetValue<int>();
                _fastForward.Invoke(inst, new object[] { target });
                return Ok(JsonValue.Create(true));
            }

            case "rng_next_item":
            {
                var inst = GetInstance(p);
                var items = p["items"]!.AsArray()
                    .Select(n => n!.GetValue<int>()).ToList();
                var nextItemInt = _nextItemGeneric.MakeGenericMethod(typeof(int));
                var picked = (int)nextItemInt.Invoke(inst, new object[] { items })!;
                return Ok(JsonValue.Create(picked));
            }

            case "rng_weighted_next_item":
            {
                var inst = GetInstance(p);
                var items = p["items"]!.AsArray()
                    .Select(n => n!.GetValue<int>()).ToList();
                var weightsArr = p["weights"]!.AsArray()
                    .Select(n => BitConverter.Int32BitsToSingle(n!.GetValue<int>()))
                    .ToArray();
                if (items.Count != weightsArr.Length)
                    throw new InvalidOperationException(
                        $"items/weights length mismatch ({items.Count} vs {weightsArr.Length})");
                // Build a lookup so Func<int, float> works for arbitrary item
                // values. (For testing, items are usually 0..N-1 anyway.)
                var dict = new Dictionary<int, float>();
                for (var i = 0; i < items.Count; i++) dict[items[i]] = weightsArr[i];
                Func<int, float> weightFn = x => dict[x];
                var weightedInt = _weightedNextItemGeneric.MakeGenericMethod(typeof(int));
                var picked = (int)weightedInt.Invoke(inst, new object[] { items, weightFn })!;
                return Ok(JsonValue.Create(picked));
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

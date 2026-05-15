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
                response = new JsonObject
                {
                    ["error"] = ex.InnerException.Message,
                    ["trace"] = ex.InnerException.StackTrace,
                };
            }
            catch (Exception ex)
            {
                response = new JsonObject
                {
                    ["error"] = ex.Message,
                    ["trace"] = ex.StackTrace,
                };
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
    private readonly Dictionary<string, ActReflectionBundle> _actBundles;
    private readonly StandardActMapReflectionBundle _samBundle;
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
    private readonly CombatReflectionBundle _combat;

    private Dispatcher(
        Type rngType,
        ConstructorInfo ctor,
        ConstructorInfo ctorNamed,
        MethodInfo stringHelperHash,
        MethodInfo stringHelperSnake,
        MethodInfo listExtStableShuffle,
        Dictionary<string, ActReflectionBundle> actBundles,
        StandardActMapReflectionBundle samBundle,
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
        PropertyInfo seedProp,
        CombatReflectionBundle combat)
    {
        _rngType = rngType;
        _rngCtor = ctor;
        _rngCtorNamed = ctorNamed;
        _stringHelperHash = stringHelperHash;
        _stringHelperSnake = stringHelperSnake;
        _listExtStableShuffle = listExtStableShuffle;
        _actBundles = actBundles;
        _samBundle = samBundle;
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
        _combat = combat;
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

        // Concrete acts. We bypass ActModel's constructor (which depends on
        // AbstractModel/ModelDb initialization that isn't safe to trigger
        // here) and use uninitialized instances — GetMapPointTypes only
        // reads the rng parameter, no instance state.
        var countsType = asm.GetType("MegaCrit.Sts2.Core.Map.MapPointTypeCounts",
            throwOnError: true)!;
        var countsNumElites = countsType.GetProperty("NumOfElites")
            ?? throw new InvalidOperationException("NumOfElites not found");
        var countsNumShops = countsType.GetProperty("NumOfShops")
            ?? throw new InvalidOperationException("NumOfShops not found");
        var countsNumUnknowns = countsType.GetProperty("NumOfUnknowns")
            ?? throw new InvalidOperationException("NumOfUnknowns not found");
        var countsNumRests = countsType.GetProperty("NumOfRests")
            ?? throw new InvalidOperationException("NumOfRests not found");

        var actNames = new[] { "Overgrowth", "Hive", "Glory", "Underdocks", "DeprecatedAct" };
        var actBundles = new Dictionary<string, ActReflectionBundle>();
        foreach (var name in actNames)
        {
            var actType = asm.GetType($"MegaCrit.Sts2.Core.Models.Acts.{name}",
                throwOnError: true)!;
            var instance = System.Runtime.CompilerServices.RuntimeHelpers
                .GetUninitializedObject(actType);
            var getMapPointTypes = actType.GetMethod("GetMapPointTypes")
                ?? throw new InvalidOperationException(
                    $"{name}.GetMapPointTypes not found");
            actBundles[name] = new ActReflectionBundle(
                instance, getMapPointTypes,
                countsNumElites, countsNumShops, countsNumUnknowns, countsNumRests);
        }

        // StandardActMap construction + grid inspection bundle.
        var samType = asm.GetType("MegaCrit.Sts2.Core.Map.StandardActMap",
            throwOnError: true)!;
        var actModelType = asm.GetType("MegaCrit.Sts2.Core.Models.ActModel",
            throwOnError: true)!;
        var samCtor = samType.GetConstructor(new[] {
            rngType, actModelType, typeof(bool), typeof(bool),
            typeof(bool), countsType, typeof(bool),
        }) ?? throw new InvalidOperationException("StandardActMap ctor not found");
        var samGrid = samType.GetProperty("Grid",
            BindingFlags.NonPublic | BindingFlags.Instance)
            ?? throw new InvalidOperationException("Grid property not found");
        var samBoss = samType.GetProperty("BossMapPoint")
            ?? throw new InvalidOperationException("BossMapPoint not found");
        var samStarting = samType.GetProperty("StartingMapPoint")
            ?? throw new InvalidOperationException("StartingMapPoint not found");
        var mapPointType = asm.GetType("MegaCrit.Sts2.Core.Map.MapPoint",
            throwOnError: true)!;
        var mpCoord = mapPointType.GetField("coord")
            ?? throw new InvalidOperationException("MapPoint.coord not found");
        var mpPointType = mapPointType.GetProperty("PointType")
            ?? throw new InvalidOperationException("MapPoint.PointType not found");
        var mpChildren = mapPointType.GetProperty("Children")
            ?? throw new InvalidOperationException("MapPoint.Children not found");
        var mpParents = mapPointType.GetField("parents")
            ?? throw new InvalidOperationException("MapPoint.parents not found");
        var mapCoordType = asm.GetType("MegaCrit.Sts2.Core.Map.MapCoord",
            throwOnError: true)!;
        var mcCol = mapCoordType.GetField("col")
            ?? throw new InvalidOperationException("MapCoord.col not found");
        var mcRow = mapCoordType.GetField("row")
            ?? throw new InvalidOperationException("MapCoord.row not found");

        var samBundle = new StandardActMapReflectionBundle(
            samCtor, samGrid, samBoss, samStarting,
            mpCoord, mpPointType, mpChildren, mpParents,
            mcCol, mcRow);

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

        var combatBundle = CombatReflectionBundle.Build(asm);

        return new Dispatcher(rngType, ctor, ctorNamed,
            stringHelperHash, stringHelperSnake, listExtStableShuffle, actBundles, samBundle,
            nextIntSingle, nextIntRange,
            nextBool, nextDoubleSingle, nextDoubleRange,
            nextFloatSingle, nextFloatRange, nextUIntSingle, nextUIntRange,
            nextGaussianDouble, nextGaussianFloat, nextGaussianInt,
            fastForward, shuffleGen, nextItemGen, weightedNextItemGen,
            counter, seed, combatBundle);
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

            case "act_get_map_point_types":
            {
                var actName = p["act"]!.GetValue<string>();
                if (!_actBundles.TryGetValue(actName, out var bundle))
                    throw new InvalidOperationException($"unknown act: {actName}");
                var rngInst = GetInstance(p);
                var counts = bundle.GetMapPointTypes.Invoke(
                    bundle.Instance, new object[] { rngInst })!;
                return Ok(new JsonObject
                {
                    ["num_of_unknowns"] = (int)bundle.NumOfUnknowns.GetValue(counts)!,
                    ["num_of_rests"] = (int)bundle.NumOfRests.GetValue(counts)!,
                    ["num_of_shops"] = (int)bundle.NumOfShops.GetValue(counts)!,
                    ["num_of_elites"] = (int)bundle.NumOfElites.GetValue(counts)!,
                });
            }

            case "standard_act_map_construct":
            {
                var actName = p["act"]!.GetValue<string>();
                if (!_actBundles.TryGetValue(actName, out var actBundle))
                    throw new InvalidOperationException($"unknown act: {actName}");
                var rngInst = GetInstance(p);
                var isMultiplayer = p["is_multiplayer"]?.GetValue<bool>() ?? false;
                var replaceTreasure = p["replace_treasure_with_elites"]?.GetValue<bool>() ?? false;
                var hasSecondBoss = p["has_second_boss"]?.GetValue<bool>() ?? false;
                var enablePruning = p["enable_pruning"]?.GetValue<bool>() ?? false;
                var sam = _samBundle.Ctor.Invoke(new object?[] {
                    rngInst, actBundle.Instance, isMultiplayer, replaceTreasure,
                    hasSecondBoss, null, enablePruning,
                })!;
                return Ok(SerializeMap(sam, _samBundle));
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

            // ========================================================
            // Phase 1: Combat scaffold. Mock-combat construction and
            // state-dump RPCs. No card execution yet — that's Phase 2.
            // ========================================================
            case "combat_new":
            {
                // CombatState(EncounterModel=null, IRunState=null, ...).
                // NullRunState.Instance is the default fallback. We pass
                // explicit nulls for all optional params.
                var inst = _combat.CombatStateCtor.Invoke(
                    new object?[] { null, null, null, null });
                var handle = _nextHandle++;
                _instances[handle] = inst!;
                return Ok(JsonValue.Create(handle));
            }

            case "combat_add_player":
            {
                var combat = GetInstance(p);
                var characterId = p["character_id"]!.GetValue<string>();
                var seed = (uint)(p["seed"]?.GetValue<long>() ?? 0L);
                var unlock = _combat.UnlockNone;
                var characterModel = _combat.GetCharacterById(characterId);
                // Player.CreateForNewRun(character, unlockState, netId)
                var player = _combat.PlayerCreateForNewRun.Invoke(
                    null, new object[] { characterModel, unlock, 0UL })!;
                _combat.AddPlayer.Invoke(combat, new[] { player });
                // ResetCombatState + PopulateCombatState wire the
                // combat-frame PlayerCombatState (hand/draw/discard/
                // exhaust piles) and seed the draw pile from the
                // master deck via a deterministic Rng.
                _combat.ResetCombatState.Invoke(player, null);
                var rng = _rngCtor.Invoke(new object[] { seed, 0 });
                _combat.PopulateCombatState.Invoke(player, new[] { rng, combat });
                // PopulateCombatState uses state.CloneCard which calls
                // the 1-arg AddCard (no Owner set). Walk the draw pile
                // and assign Owner on each cloned card so
                // CombatState.Contains doesn't NRE on the
                // Owner.IsActiveForHooks deref.
                var pcs = _combat.PlayerCombatState.GetValue(player)!;
                var drawPile = _combat.PcsDrawPile.GetValue(pcs)!;
                var drawCards = (System.Collections.IList)_combat.PileCards.GetValue(drawPile)!;
                var ownerProp = _combat.CardModelType.GetProperty("Owner")
                    ?? throw new InvalidOperationException("CardModel.Owner not found");
                foreach (var c in drawCards)
                {
                    if (c == null) continue;
                    if (ownerProp.GetValue(c) == null) ownerProp.SetValue(c, player);
                }
                return Ok(JsonValue.Create(true));
            }

            case "combat_add_enemy":
            {
                var combat = GetInstance(p);
                var monsterId = p["monster_id"]!.GetValue<string>();
                var slot = p["slot"]?.GetValue<string?>();
                var monsterModel = _combat.GetMonsterById(monsterId);
                // monsterModel.ToMutable()
                var mutable = _combat.ToMutable.Invoke(monsterModel, null)!;
                // combatState.CreateCreature(monster, CombatSide.Enemy, slot).
                // CombatSide enum: None=0, Player=1, Enemy=2.
                var enemySide = Enum.ToObject(_combat.CombatSideType, 2);
                var creature = _combat.CreateCreature.Invoke(
                    combat, new object?[] { mutable, enemySide, slot });
                _combat.AddCreature.Invoke(combat, new[] { creature });
                return Ok(JsonValue.Create(true));
            }

            case "combat_dump":
            {
                var combat = GetInstance(p);
                return Ok(SerializeCombat(combat));
            }

            case "combat_init_run_state":
            {
                // Upgrade the player's RunState from NullRunState to a
                // real one via RunState.CreateForTest. The setter on
                // Player.RunState requires the current to be
                // NullRunState (one-time transition), so this must run
                // exactly once per player. Unblocks Ancient relics
                // whose AfterObtained reaches into RunState
                // (CreateCard<T>, UnlockState, CardMultiplayerConstraint,
                // CurrentMapPointHistoryEntry, etc.).
                var combat = GetInstance(p);
                var playerIdx = p["player_idx"]?.GetValue<int>() ?? 0;
                var seed = p["seed"]?.GetValue<string?>();
                var allies = (System.Collections.IList)_combat.Allies.GetValue(combat)!;
                var ownerCreature = allies[playerIdx]!;
                var ownerPlayer = _combat.CreaturePlayer.GetValue(ownerCreature)!;
                try
                {
                    var playerListType = typeof(System.Collections.Generic.List<>)
                        .MakeGenericType(_combat.CombatStateAddCardWithOwner.GetParameters()[1].ParameterType);
                    var playerList = (System.Collections.IList)Activator.CreateInstance(playerListType)!;
                    playerList.Add(ownerPlayer);
                    // Optional args: acts=null, modifiers=null, gameMode=Standard,
                    // ascensionLevel=0, seed.
                    var args = new object?[] {
                        playerList, null, null,
                        Enum.ToObject(_combat.GameModeType, 0),
                        0, seed
                    };
                    _combat.RunStateCreateForTest.Invoke(null, args);
                }
                catch (Exception ex)
                {
                    return new JsonObject {
                        ["error"] = new JsonObject {
                            ["code"] = -32000,
                            ["message"] = $"init_run_state: {ex.InnerException?.Message ?? ex.Message}",
                        },
                    };
                }
                return Ok(JsonValue.Create(true));
            }

            case "combat_grant_relic":
            {
                var combat = GetInstance(p);
                var relicId = p["relic_id"]!.GetValue<string>();
                var playerIdx = p["player_idx"]?.GetValue<int>() ?? 0;
                var allies = (System.Collections.IList)_combat.Allies.GetValue(combat)!;
                var ownerCreature = allies[playerIdx]!;
                var ownerPlayer = _combat.CreaturePlayer.GetValue(ownerCreature)!;
                var canonicalRelic = _combat.GetRelicById(relicId);
                var mutableRelic = _combat.RelicToMutable.Invoke(canonicalRelic, null)!;
                // RelicCmd.Obtain handles AddRelicInternal + AfterObtained.
                // Returns a Task; await synchronously via .GetAwaiter().GetResult().
                try
                {
                    var task = _combat.RelicCmdObtain.Invoke(null,
                        new object[] { mutableRelic, ownerPlayer, -1 })!;
                    var awaiter = task.GetType().GetMethod("GetAwaiter")!
                        .Invoke(task, null)!;
                    awaiter.GetType().GetMethod("GetResult")!.Invoke(awaiter, null);
                }
                catch (Exception ex)
                {
                    return new JsonObject {
                        ["error"] = new JsonObject {
                            ["code"] = -32000,
                            ["message"] = $"grant_relic({relicId}): {ex.InnerException?.Message ?? ex.Message}",
                        },
                    };
                }
                return Ok(JsonValue.Create(true));
            }

            case "combat_fire_before_combat_start":
            {
                // Fire the two "combat start" hooks the rust simulator
                // collapses under `RelicHook::BeforeCombatStart`:
                //   - C# `RelicModel.BeforeCombatStart()`
                //   - C# `RelicModel.AfterRoomEntered(CombatRoom)`
                // Different C# relics override different hooks (Anchor
                // → BeforeCombatStart; BronzeScales / DataDisk / RedSkull
                // / Vajra / OddlySmoothStone → AfterRoomEntered) but
                // both fire at functionally the same moment.
                // Hook.* statics are no-op'd in this harness; this RPC
                // restores the relic-relevant slice without re-enabling
                // the listener-iteration code paths that NRE on partial
                // model init.
                var combat = GetInstance(p);
                var playerIdx = p["player_idx"]?.GetValue<int>() ?? 0;
                var allies = (System.Collections.IList)_combat.Allies.GetValue(combat)!;
                var ownerCreature = allies[playerIdx]!;
                var ownerPlayer = _combat.CreaturePlayer.GetValue(ownerCreature)!;
                var relics = (System.Collections.IEnumerable?)_combat.PlayerRelics.GetValue(ownerPlayer);
                if (relics == null)
                {
                    return Ok(JsonValue.Create(true));
                }
                var fakeRoom = _combat.CombatRoomFromState.Invoke(new[] { combat })!;
                var errors = new JsonArray();
                void FireHook(MethodInfo method, object relic, object?[]? args)
                {
                    try
                    {
                        var task = method.Invoke(relic, args)!;
                        var awaiter = task.GetType().GetMethod("GetAwaiter")!
                            .Invoke(task, null)!;
                        awaiter.GetType().GetMethod("GetResult")!.Invoke(awaiter, null);
                    }
                    catch (Exception ex)
                    {
                        var id = _combat.AbstractModelId.GetValue(relic);
                        errors.Add($"{method.Name}({id}): {ex.InnerException?.Message ?? ex.Message}");
                    }
                }
                foreach (var r in relics)
                {
                    if (r == null) continue;
                    FireHook(_combat.BeforeCombatStartMethod, r, null);
                    FireHook(_combat.AfterRoomEnteredMethod, r, new object?[] { fakeRoom });
                }
                var result = new JsonObject();
                result["ok"] = JsonValue.Create(true);
                result["errors"] = errors;
                return Ok(result);
            }

            case "combat_force_card_to_hand":
            {
                var combat = GetInstance(p);
                var cardId = p["card_id"]!.GetValue<string>();
                var upgrade = p["upgrade_level"]?.GetValue<int>() ?? 0;
                var playerIdx = p["player_idx"]?.GetValue<int>() ?? 0;
                // Walk allies → grab the playerIdx-th player creature.
                var allies = (System.Collections.IList)_combat.Allies.GetValue(combat)!;
                var ownerCreature = allies[playerIdx]!;
                var ownerPlayer = _combat.CreaturePlayer.GetValue(ownerCreature)!;
                var canonicalCard = _combat.GetCardById(cardId);
                var mutableCard = _combat.CardToMutable.Invoke(canonicalCard, null)!;
                // Upgrade N times.
                for (int i = 0; i < upgrade; i++)
                {
                    var upgradeInternal = _combat.CardModelType.GetMethod(
                        "UpgradeInternal",
                        BindingFlags.Public | BindingFlags.Instance);
                    upgradeInternal?.Invoke(mutableCard, null);
                }
                // state.AddCard(card, owner) — sets Owner + registers in _allCards.
                _combat.CombatStateAddCardWithOwner.Invoke(combat,
                    new[] { mutableCard, ownerPlayer });
                // Add to PlayerCombatState.Hand.
                var pcs = _combat.PlayerCombatState.GetValue(ownerPlayer)!;
                var hand = _combat.PcsHand.GetValue(pcs)!;
                _combat.PileAddInternal.Invoke(hand,
                    new object[] { mutableCard, -1, true });
                return Ok(JsonValue.Create(true));
            }

            case "combat_play_card":
            {
                var combat = GetInstance(p);
                var handIdx = p["hand_idx"]!.GetValue<int>();
                var playerIdx = p["player_idx"]?.GetValue<int>() ?? 0;
                var targetIdx = p["target_idx"]?.GetValue<int?>();
                var allyTargetIdx = p["ally_target_idx"]?.GetValue<int?>();
                var allies = (System.Collections.IList)_combat.Allies.GetValue(combat)!;
                var ownerCreature = allies[playerIdx]!;
                var ownerPlayer = _combat.CreaturePlayer.GetValue(ownerCreature)!;
                var pcs = _combat.PlayerCombatState.GetValue(ownerPlayer)!;
                var hand = _combat.PcsHand.GetValue(pcs)!;
                var play = _combat.PcsPlayPile.GetValue(pcs)!;
                var handCards = (System.Collections.IList)_combat.PileCards.GetValue(hand)!;
                if (handIdx < 0 || handIdx >= handCards.Count)
                {
                    return new JsonObject { ["error"] = $"hand index {handIdx} out of range (size {handCards.Count})" };
                }
                var card = handCards[handIdx]!;
                // Resolve target Creature. target_idx is enemies index;
                // ally_target_idx is allies index (for AnyAlly cards).
                object? target = null;
                if (allyTargetIdx is int at && at >= 0 && at < allies.Count)
                {
                    target = allies[at]!;
                }
                else if (targetIdx is int t)
                {
                    var enemies = (System.Collections.IList)_combat.Enemies.GetValue(combat)!;
                    if (t >= 0 && t < enemies.Count) target = enemies[t]!;
                }
                // Hand → PlayPile.
                _combat.PileRemoveInternal.Invoke(hand, new object[] { card, true });
                _combat.PileAddInternal.Invoke(play, new object[] { card, -1, true });
                // Build CardPlay { Card=card, Target=target, ... }.
                var cardPlay = _combat.CardPlayCtor.Invoke(null)!;
                cardPlayType_set(cardPlay, "Card", card);
                if (target != null) cardPlayType_set(cardPlay, "Target", target);
                cardPlayType_set(cardPlay, "IsAutoPlay", true);
                cardPlayType_set(cardPlay, "PlayIndex", 0);
                cardPlayType_set(cardPlay, "PlayCount", 1);
                // Resolve target pile via GetResultPileType (protected).
                var resultPileVal = _combat.CardGetResultPileType.Invoke(card, null);
                cardPlayType_set(cardPlay, "ResultPile", resultPileVal!);
                // Invoke OnPlay (protected, returns Task). The async
                // body can throw via the Task; GetResult() unwraps.
                string? onPlayError = null;
                string? onPlayTrace = null;
                try
                {
                    var task = (Task?)_combat.CardOnPlay.Invoke(card,
                        new object?[] { null, cardPlay });
                    task?.GetAwaiter().GetResult();
                }
                catch (TargetInvocationException ex) when (ex.InnerException is not null)
                {
                    onPlayError = ex.InnerException.Message;
                    onPlayTrace = ex.InnerException.StackTrace;
                }
                catch (Exception ex)
                {
                    onPlayError = ex.Message;
                    onPlayTrace = ex.StackTrace;
                }
                // Route Play → result pile (always — partial plays still
                // route).
                _combat.PileRemoveInternal.Invoke(play, new object[] { card, true });
                var dest = ResolvePileObject(pcs, resultPileVal);
                if (dest != null)
                    _combat.PileAddInternal.Invoke(dest, new object[] { card, -1, true });
                var resp = new JsonObject { ["result"] = onPlayError == null };
                if (onPlayError != null)
                {
                    resp["onplay_error"] = onPlayError;
                    resp["onplay_trace"] = onPlayTrace;
                }
                return resp;
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

    private JsonObject SerializeCombat(object combat)
    {
        var round = (int)_combat.RoundNumber.GetValue(combat)!;
        var side = (int)Convert.ChangeType(
            _combat.CurrentSide.GetValue(combat)!, typeof(int));
        var allies = (System.Collections.IEnumerable)_combat.Allies.GetValue(combat)!;
        var enemies = (System.Collections.IEnumerable)_combat.Enemies.GetValue(combat)!;
        var alliesArr = new JsonArray();
        foreach (var c in allies) alliesArr.Add(SerializeCreature(c));
        var enemiesArr = new JsonArray();
        foreach (var c in enemies) enemiesArr.Add(SerializeCreature(c));
        return new JsonObject
        {
            ["round_number"] = round,
            ["current_side"] = side,
            ["allies"] = alliesArr,
            ["enemies"] = enemiesArr,
        };
    }

    private JsonObject SerializeCreature(object creature)
    {
        string? name = null;
        try { name = (string?)_combat.CreatureName.GetValue(creature); }
        catch { }
        var hp = (int)_combat.CreatureCurrentHp.GetValue(creature)!;
        var maxHp = (int)_combat.CreatureMaxHp.GetValue(creature)!;
        var block = (int)_combat.CreatureBlock.GetValue(creature)!;
        var isPlayer = (bool)_combat.CreatureIsPlayer.GetValue(creature)!;
        var powers = SerializePowers(creature);
        var creatureNode = new JsonObject
        {
            ["name"] = name,
            ["current_hp"] = hp,
            ["max_hp"] = maxHp,
            ["block"] = block,
            ["is_player"] = isPlayer,
            ["powers"] = powers,
        };
        if (isPlayer)
        {
            try
            {
                var player = _combat.CreaturePlayer.GetValue(creature);
                if (player != null)
                {
                    creatureNode["player"] = SerializePlayer(player);
                }
            }
            catch (Exception ex)
            {
                creatureNode["player_error"] = ex.Message;
            }
        }
        return creatureNode;
    }

    private JsonArray SerializePowers(object creature)
    {
        var powers = (System.Collections.IEnumerable?)_combat.CreaturePowers.GetValue(creature);
        if (powers == null) return new JsonArray();
        // Collect then sort by id to normalize against the Rust dump
        // (which sorts at emit time).
        var tmp = new List<(string id, int amount)>();
        foreach (var p in powers)
        {
            var id = ModelIdString(_combat.AbstractModelId.GetValue(p)) ?? "";
            var amount = (int)_combat.PowerAmount.GetValue(p)!;
            tmp.Add((id, amount));
        }
        tmp.Sort((a, b) => string.CompareOrdinal(a.id, b.id));
        var arr = new JsonArray();
        foreach (var (id, amount) in tmp)
        {
            arr.Add(new JsonObject { ["id"] = id, ["amount"] = amount });
        }
        return arr;
    }

    private JsonObject SerializePlayer(object player)
    {
        var node = new JsonObject();
        try
        {
            node["max_energy_base"] = (int)_combat.PlayerMaxEnergy.GetValue(player)!;
        }
        catch (Exception ex) { node["max_energy_error"] = ex.Message; }
        try
        {
            var pcs = _combat.PlayerCombatState.GetValue(player);
            if (pcs != null)
            {
                try { node["energy"] = (int)_combat.PcsEnergy.GetValue(pcs)!; }
                catch (Exception ex) { node["energy_error"] = ex.Message; }
                try { node["stars"] = (int)_combat.PcsStars.GetValue(pcs)!; }
                catch (Exception ex) { node["stars_error"] = ex.Message; }
                node["hand"] = SerializePile(_combat.PcsHand.GetValue(pcs));
                node["draw"] = SerializePile(_combat.PcsDrawPile.GetValue(pcs));
                node["discard"] = SerializePile(_combat.PcsDiscardPile.GetValue(pcs));
                node["exhaust"] = SerializePile(_combat.PcsExhaustPile.GetValue(pcs));
                node["play"] = SerializePile(_combat.PcsPlayPile.GetValue(pcs));
            }
        }
        catch (Exception ex)
        {
            node["pcs_error"] = ex.Message;
        }
        try
        {
            node["master_deck"] = SerializePile(_combat.PlayerMasterDeck.GetValue(player));
        }
        catch (Exception ex) { node["master_deck_error"] = ex.Message; }
        try
        {
            var relicsArr = new JsonArray();
            var relics = (System.Collections.IEnumerable?)_combat.PlayerRelics.GetValue(player);
            if (relics != null)
            {
                foreach (var r in relics)
                {
                    relicsArr.Add(ModelIdString(_combat.AbstractModelId.GetValue(r)));
                }
            }
            node["relics"] = relicsArr;
        }
        catch (Exception ex) { node["relics_error"] = ex.Message; }
        try
        {
            var potionsArr = new JsonArray();
            var potions = (System.Collections.IEnumerable?)_combat.PlayerPotionSlots.GetValue(player);
            if (potions != null)
            {
                foreach (var p in potions)
                {
                    if (p == null) { potionsArr.Add((JsonNode?)null); continue; }
                    potionsArr.Add(ModelIdString(_combat.AbstractModelId.GetValue(p)));
                }
            }
            node["potions"] = potionsArr;
        }
        catch (Exception ex) { node["potions_error"] = ex.Message; }
        return node;
    }

    private JsonArray SerializePile(object? pile)
    {
        if (pile == null) return new JsonArray();
        var cards = (System.Collections.IEnumerable?)_combat.PileCards.GetValue(pile);
        if (cards == null) return new JsonArray();
        // Sort pile contents by id so parity diffs ignore within-pile
        // ordering. Rust pops from end-of-Vec, C# from index 0 — the
        // conventions are inverse, so without sorting the same card
        // movement looks like a divergence.
        var tmp = new List<(string key, JsonObject node)>();
        foreach (var c in cards)
        {
            var node = SerializeCard(c);
            var key = (node["id"]?.GetValue<string>() ?? "") + "/" +
                      (node["upgrade_level"]?.GetValue<int>().ToString() ?? "0");
            tmp.Add((key, node));
        }
        tmp.Sort((a, b) => string.CompareOrdinal(a.key, b.key));
        var arr = new JsonArray();
        foreach (var (_, node) in tmp) arr.Add(node);
        return arr;
    }

    private JsonObject SerializeCard(object card)
    {
        var id = ModelIdString(_combat.AbstractModelId.GetValue(card));
        var upgrade = (int)_combat.CardCurrentUpgradeLevel.GetValue(card)!;
        var node = new JsonObject { ["id"] = id, ["upgrade_level"] = upgrade };
        try
        {
            var ench = _combat.CardEnchantment.GetValue(card);
            if (ench != null)
            {
                var enchId = ModelIdString(_combat.AbstractModelId.GetValue(ench));
                var amount = (int)_combat.EnchantmentAmountField.GetValue(ench)!;
                node["enchantment"] = new JsonObject
                {
                    ["id"] = enchId,
                    ["amount"] = amount,
                };
            }
        }
        catch { /* enchantment absent or unreadable */ }
        return node;
    }

    private static string? ModelIdString(object? mid)
    {
        if (mid == null) return null;
        // ModelId.ToString() returns "CATEGORY.ENTRY".
        return mid.ToString();
    }

    private void cardPlayType_set(object cardPlay, string propName, object value)
    {
        var prop = cardPlay.GetType().GetProperty(propName)
            ?? throw new InvalidOperationException(
                $"CardPlay.{propName} not found");
        prop.SetValue(cardPlay, value);
    }

    /// Map a PileType enum value to the CardPile object on the given
    /// PlayerCombatState. `pileType` arg is the boxed enum from
    /// `CardModel.GetResultPileType()`.
    private object? ResolvePileObject(object pcs, object? pileType)
    {
        if (pileType == null) return null;
        // PileType: None=0, Hand=1, Draw=2, Discard=3, Exhaust=4, Play=5, Deck=6
        // (we don't rely on numeric values — use Enum.GetName).
        var name = Enum.GetName(pileType.GetType(), pileType);
        return name switch
        {
            "Hand" => _combat.PcsHand.GetValue(pcs),
            "Draw" => _combat.PcsDrawPile.GetValue(pcs),
            "Discard" => _combat.PcsDiscardPile.GetValue(pcs),
            "Exhaust" => _combat.PcsExhaustPile.GetValue(pcs),
            "Play" => _combat.PcsPlayPile.GetValue(pcs),
            _ => null,  // None / Deck / unknown → no routing.
        };
    }

    private static JsonObject Ok(JsonNode? result) => new() { ["result"] = result };

    private static JsonObject SerializeMap(object sam, StandardActMapReflectionBundle b)
    {
        var grid = (Array)b.Grid.GetValue(sam)!;
        var cols = grid.GetLength(0);
        var rows = grid.GetLength(1);
        var points = new JsonArray();
        for (var col = 0; col < cols; col++)
        for (var row = 0; row < rows; row++)
        {
            var mp = grid.GetValue(col, row);
            if (mp is null) continue;
            points.Add(SerializeMapPoint(mp, b));
        }
        var bossMp = b.Boss.GetValue(sam)!;
        var startMp = b.Starting.GetValue(sam)!;
        return new JsonObject
        {
            ["cols"] = cols,
            ["rows"] = rows,
            ["grid_points"] = points,
            ["boss"] = SerializeMapPoint(bossMp, b),
            ["starting"] = SerializeMapPoint(startMp, b),
        };
    }

    private static JsonObject SerializeMapPoint(object mp, StandardActMapReflectionBundle b)
    {
        var coord = b.MpCoord.GetValue(mp)!;
        var col = (int)b.McCol.GetValue(coord)!;
        var row = (int)b.McRow.GetValue(coord)!;
        var pt = (int)b.MpPointType.GetValue(mp)!;
        var children = (System.Collections.IEnumerable)b.MpChildren.GetValue(mp)!;
        var parents = (System.Collections.IEnumerable)b.MpParents.GetValue(mp)!;
        var childCoords = new JsonArray();
        foreach (var child in children)
        {
            var childCoord = b.MpCoord.GetValue(child)!;
            childCoords.Add(new JsonObject
            {
                ["col"] = (int)b.McCol.GetValue(childCoord)!,
                ["row"] = (int)b.McRow.GetValue(childCoord)!,
            });
        }
        var parentCoords = new JsonArray();
        foreach (var par in parents)
        {
            var parCoord = b.MpCoord.GetValue(par)!;
            parentCoords.Add(new JsonObject
            {
                ["col"] = (int)b.McCol.GetValue(parCoord)!,
                ["row"] = (int)b.McRow.GetValue(parCoord)!,
            });
        }
        return new JsonObject
        {
            ["col"] = col,
            ["row"] = row,
            ["point_type"] = pt,
            ["children"] = childCoords,
            ["parents"] = parentCoords,
        };
    }
}

internal sealed record ActReflectionBundle(
    object Instance,
    MethodInfo GetMapPointTypes,
    PropertyInfo NumOfElites,
    PropertyInfo NumOfShops,
    PropertyInfo NumOfUnknowns,
    PropertyInfo NumOfRests);

internal sealed record StandardActMapReflectionBundle(
    ConstructorInfo Ctor,
    PropertyInfo Grid,
    PropertyInfo Boss,
    PropertyInfo Starting,
    FieldInfo MpCoord,
    PropertyInfo MpPointType,
    PropertyInfo MpChildren,
    FieldInfo MpParents,
    FieldInfo McCol,
    FieldInfo McRow);

/// Holds Harmony patches that bypass the game's Godot-dependent
/// singletons. Each patch is registered once at dispatcher startup.
internal static class GodotBypass
{
    private static bool _applied;
    public static object? UninitializedSaveManager;

    public static void Apply(Assembly asm)
    {
        if (_applied) return;
        _applied = true;

        // Pre-build an uninitialized SaveManager so the patched
        // get_Instance can hand it out. Callers can then invoke
        // instance methods (we patch the specific ones that touch
        // Godot/I-O state).
        var smType = asm.GetType("MegaCrit.Sts2.Core.Saves.SaveManager",
            throwOnError: true)!;
        UninitializedSaveManager =
            System.Runtime.CompilerServices.RuntimeHelpers.GetUninitializedObject(smType);

        var harmonyAsm = AssemblyLoadContext.GetLoadContext(asm)!
            .LoadFromAssemblyName(new AssemblyName("0Harmony"));
        var harmonyType = harmonyAsm.GetType("HarmonyLib.Harmony", throwOnError: true)!;
        var harmonyCtor = harmonyType.GetConstructor(new[] { typeof(string) })!;
        var harmony = harmonyCtor.Invoke(new object[] { "sts2-sim.oracle" })!;
        var patchMethod = harmonyType.GetMethod("Patch",
            BindingFlags.Public | BindingFlags.Instance)!;
        var hmType = harmonyAsm.GetType("HarmonyLib.HarmonyMethod", throwOnError: true)!;
        var hmCtor = hmType.GetConstructor(new[] { typeof(MethodInfo) })!;

        HarmonyPatchPrefix(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Saves.SaveManager", "get_Instance",
            BindingFlags.Public | BindingFlags.Static,
            typeof(SaveManagerPrefix), nameof(SaveManagerPrefix.GetInstancePrefix));
        // No-op prefixes for SaveManager methods that touch I/O / Godot
        // state. Each prefix returns false to skip the original body.
        HarmonyPatchPrefix(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Saves.SaveManager", "MarkRelicAsSeen",
            BindingFlags.Public | BindingFlags.Instance,
            typeof(SaveManagerPrefix), nameof(SaveManagerPrefix.NoOp));
        HarmonyPatchPrefix(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Saves.SaveManager", "MarkPotionAsSeen",
            BindingFlags.Public | BindingFlags.Instance,
            typeof(SaveManagerPrefix), nameof(SaveManagerPrefix.NoOp));
        HarmonyPatchPrefix(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Saves.SaveManager", "MarkCardAsSeen",
            BindingFlags.Public | BindingFlags.Instance,
            typeof(SaveManagerPrefix), nameof(SaveManagerPrefix.NoOp));
        // SaveManager.Progress reads `_progressSaveManager.Progress`,
        // both null on our uninitialized SM. Patch the getter to
        // return an uninitialized ProgressState; downstream method
        // calls on it are patched below.
        var psType = asm.GetType("MegaCrit.Sts2.Core.Saves.ProgressState",
            throwOnError: true)!;
        SaveManagerPrefix.UninitializedProgressState =
            System.Runtime.CompilerServices.RuntimeHelpers.GetUninitializedObject(psType);
        HarmonyPatchPrefix(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Saves.SaveManager", "get_Progress",
            BindingFlags.Public | BindingFlags.Instance,
            typeof(SaveManagerPrefix), nameof(SaveManagerPrefix.GetProgressPrefix));
        HarmonyPatchPrefix(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Saves.ProgressState", "GetStatsForCharacter",
            BindingFlags.Public | BindingFlags.Instance,
            typeof(SaveManagerPrefix), nameof(SaveManagerPrefix.NullObjectPrefix));
        // Hook.ModifyShuffleOrder iterates CombatState.IterateHookListeners,
        // which calls CombatState.Contains on each candidate. Contains
        // dereferences `cardModel.Owner.IsActiveForHooks` — but cards
        // cloned via state.CloneCard(card) (called inside
        // PopulateCombatState) never get Owner assigned, so this NREs.
        // Patching ModifyShuffleOrder to no-op keeps the prior
        // UnstableShuffle result intact (the only relic that would
        // listen here is FrozenEye-style; minor cost). Re-enable when
        // we have a proper Owner-setting clone path.
        HarmonyPatchPrefix(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Hooks.Hook", "ModifyShuffleOrder",
            BindingFlags.Public | BindingFlags.Static,
            typeof(SaveManagerPrefix), nameof(SaveManagerPrefix.NoOp));
        // CombatManager.IsInProgress = true (treat as if a combat is
        // active so AttackCommand.Execute / DamageCmd etc. proceed
        // instead of short-circuiting on IsOverOrEnding).
        HarmonyPatchPrefix(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Combat.CombatManager", "get_IsInProgress",
            BindingFlags.Public | BindingFlags.Instance,
            typeof(SaveManagerPrefix), nameof(SaveManagerPrefix.TruePrefix));
        HarmonyPatchPrefix(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Combat.CombatManager", "get_IsOverOrEnding",
            BindingFlags.Public | BindingFlags.Instance,
            typeof(SaveManagerPrefix), nameof(SaveManagerPrefix.FalsePrefix));
        // Logger.GetIsRunningFromGodotEditor → false (avoids
        // Godot.OS.GetCmdlineArgs native crash during static init).
        HarmonyPatchPrefix(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Logging.Logger", "GetIsRunningFromGodotEditor",
            BindingFlags.NonPublic | BindingFlags.Public
                | BindingFlags.Static | BindingFlags.Instance,
            typeof(SaveManagerPrefix), nameof(SaveManagerPrefix.FalsePrefix));
        // CreatureCmd.TriggerAnim → no-op (animations require Godot
        // scene nodes; our headless host has none).
        HarmonyPatchPrefix(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Commands.CreatureCmd", "TriggerAnim",
            BindingFlags.Public | BindingFlags.Static,
            typeof(SaveManagerPrefix), nameof(SaveManagerPrefix.CompletedTaskPrefix));
        // get_IsEnding → false; otherwise the getter triggers
        // Hook.ShouldStopCombatFromEnding which iterates hook listeners
        // and NREs.
        HarmonyPatchPrefix(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Combat.CombatManager", "get_IsEnding",
            BindingFlags.Public | BindingFlags.Instance,
            typeof(SaveManagerPrefix), nameof(SaveManagerPrefix.FalsePrefix));
        // CardPile.RandomizeOrderInternal → no-op so the draw pile
        // stays in starter-deck order. Parity-tests on the Rust side
        // don't shuffle either (the audit cares about behavior, not
        // randomness). When we add explicit RNG parity tests later,
        // we can remove this patch and seed both sides identically.
        HarmonyPatchPrefix(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Entities.Cards.CardPile", "RandomizeOrderInternal",
            BindingFlags.Public | BindingFlags.Instance,
            typeof(SaveManagerPrefix), nameof(SaveManagerPrefix.NoOp));

        // Sfx / Vfx commands rely on Godot scene nodes — no-op them.
        PatchAllStaticMethodsToNoOp(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Commands.SfxCmd");
        PatchAllStaticMethodsToNoOp(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Commands.VfxCmd");
        // Cmd.* helpers — Wait/CustomScaledWait/etc. read SaveManager.
        // PrefsSave for animation speed scaling. No-op these so we
        // don't dive into more Godot.
        PatchAllStaticMethodsToNoOp(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Commands.Cmd");
        // Log.* methods write through Godot.GD.Print (native call).
        // No-op every static log helper to keep the headless host
        // alive when game code logs informationally.
        PatchAllStaticMethodsToNoOp(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Logging.Log");
        // Hook.* iterators (BeforeAttack, BeforeDamageReceived,
        // ModifyDamage, AfterCardPlayed, etc.) walk
        // CombatState.IterateHookListeners → Contains which NREs on
        // partially-initialized models in our headless setup. No
        // relics in our scenario register these hooks, so a no-op
        // patch preserves behavior. (When we test relic parity, we'll
        // revisit this with targeted patches per hook.)
        PatchAllStaticMethodsToNoOp(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Hooks.Hook");
        // LocManager static init touches Godot resource paths. Patch
        // any static method on it that might be called during the
        // dispatch's interactive paths (Loc strings).
        PatchAllStaticMethodsToNoOp(asm, harmony, patchMethod, hmCtor,
            "MegaCrit.Sts2.Core.Localization.LocManager");
        // NullRunState.CreateCard normally throws. Replace with
        // ToMutable+Owner so RelicCmd.Obtain bodies that call
        // RunState.CreateCard<X>(player) followed by CardPileCmd.Add to
        // the master deck (JewelryBox/NeowsTorment/Storybook/DustyTome
        // /TanxsWhistle/etc.) can complete. Patches BOTH overloads
        // (generic and CardModel-arg) so callers of either succeed.
        var nullRunType = asm.GetType("MegaCrit.Sts2.Core.Runs.NullRunState",
            throwOnError: false);
        if (nullRunType != null)
        {
            var cardModelType = asm.GetType("MegaCrit.Sts2.Core.Models.CardModel",
                throwOnError: true)!;
            var playerType = asm.GetType("MegaCrit.Sts2.Core.Entities.Players.Player",
                throwOnError: true)!;
            var nullRunCreateCardCanonical = nullRunType.GetMethod("CreateCard",
                BindingFlags.Public | BindingFlags.Instance,
                new[] { cardModelType, playerType });
            if (nullRunCreateCardCanonical != null)
            {
                HarmonyPatchPrefixDirect(harmony, patchMethod, hmCtor,
                    nullRunCreateCardCanonical,
                    typeof(SaveManagerPrefix),
                    nameof(SaveManagerPrefix.NullRunCreateCardPrefix));
            }
            // Generic CreateCard<T>(Player) — Harmony 2 in this MonoMod
            // build can't patch generic method DEFINITIONS (throws
            // NotSupportedException in MMReflectionImporter on the open
            // generic parameter). Callers that hit the generic overload
            // (JewelryBox/Storybook/NeowsTorment/etc. via
            // `RunState.CreateCard<X>(player)`) will still throw
            // "Cannot create cards in a null run"; those relics remain
            // ORACLE_ERROR until a real RunState is wired (or until each
            // concrete T is enumerated and patched as a closed generic).
        }
    }

    /// Patch every public static method on `typeName` to a no-op
    /// (return Task.CompletedTask for Task-returning, do-nothing for
    /// void). Sfx/Vfx command types are essentially side-effect-only;
    /// our headless host has no Godot scene to render to.
    private static void PatchAllStaticMethodsToNoOp(
        Assembly asm, object harmony, MethodInfo patchMethod,
        ConstructorInfo hmCtor, string typeName)
    {
        var t = asm.GetType(typeName, throwOnError: false);
        if (t == null) return;
        var hmType = hmCtor.DeclaringType!;
        foreach (var m in t.GetMethods(
            BindingFlags.Public | BindingFlags.Static
                | BindingFlags.DeclaredOnly))
        {
            // Pick prefix based on return type.
            string prefixName;
            if (m.ReturnType == typeof(void))
                prefixName = nameof(SaveManagerPrefix.NoOp);
            else if (typeof(Task).IsAssignableFrom(m.ReturnType))
                prefixName = nameof(SaveManagerPrefix.CompletedTaskPrefix);
            else
                continue;  // skip non-void, non-Task methods (e.g. helpers).
            var prefix = typeof(SaveManagerPrefix).GetMethod(prefixName,
                BindingFlags.Public | BindingFlags.Static)!;
            var hm = hmCtor.Invoke(new object[] { prefix });
            var paramCount = patchMethod.GetParameters().Length;
            var args = new object?[paramCount];
            args[0] = m;
            args[1] = hm;
            for (int i = 2; i < paramCount; i++) args[i] = null;
            try { patchMethod.Invoke(harmony, args); }
            catch { /* ignore patch failures for individual methods */ }
        }
    }

    private static void HarmonyPatchPrefix(
        Assembly asm, object harmony, MethodInfo patchMethod,
        ConstructorInfo hmCtor, string typeName, string methodName,
        BindingFlags flags, Type prefixHost, string prefixName)
    {
        var target = asm.GetType(typeName, throwOnError: true)!
            .GetMethod(methodName, flags)
            ?? throw new InvalidOperationException(
                $"target {typeName}.{methodName} not found");
        HarmonyPatchPrefixDirect(harmony, patchMethod, hmCtor, target, prefixHost, prefixName);
    }

    /// Same as `HarmonyPatchPrefix` but the caller supplies the resolved
    /// MethodBase target directly (used for generic method definitions and
    /// non-trivial overload lookups).
    private static void HarmonyPatchPrefixDirect(
        object harmony, MethodInfo patchMethod, ConstructorInfo hmCtor,
        System.Reflection.MethodBase target, Type prefixHost, string prefixName)
    {
        var prefix = prefixHost.GetMethod(prefixName,
            BindingFlags.Public | BindingFlags.Static)
            ?? throw new InvalidOperationException(
                $"prefix {prefixHost.Name}.{prefixName} not found");
        var hm = hmCtor.Invoke(new object[] { prefix });
        var paramCount = patchMethod.GetParameters().Length;
        var args = new object?[paramCount];
        args[0] = target;
        args[1] = hm;
        for (int i = 2; i < paramCount; i++) args[i] = null;
        patchMethod.Invoke(harmony, args);
    }
}

internal static class SaveManagerPrefix
{
    public static object? UninitializedProgressState;

    // Return the uninitialized SaveManager. Callers do `.MarkXxxAsSeen`
    // and similar — those methods are individually patched to no-op.
    public static bool GetInstancePrefix(ref object? __result)
    {
        __result = GodotBypass.UninitializedSaveManager;
        return false;
    }
    public static bool GetProgressPrefix(ref object? __result)
    {
        __result = UninitializedProgressState;
        return false;
    }
    // Skip the original body for void-returning methods.
    public static bool NoOp() => false;
    // Skip + return null for object-returning methods.
    public static bool NullObjectPrefix(ref object? __result)
    {
        __result = null;
        return false;
    }
    public static bool TruePrefix(ref bool __result)
    {
        __result = true;
        return false;
    }
    public static bool FalsePrefix(ref bool __result)
    {
        __result = false;
        return false;
    }
    // Skip + return a completed Task for async-Task void-effect methods.
    public static bool CompletedTaskPrefix(ref Task __result)
    {
        __result = Task.CompletedTask;
        return false;
    }
    // NullRunState.CreateCard(CardModel canonical, Player owner) normally
    // throws "Cannot create cards in a null run." Replace with the same
    // body as the real RunState.CreateCard: clone the canonical to mutable
    // and return it. We skip the AddCard/AfterCreated tracking that the
    // real RunState does (run-state card-collection bookkeeping), because
    // the parity test only needs the card to exist as a CardModel that
    // CardPileCmd.Add can route into the master deck.
    public static bool NullRunCreateCardPrefix(
        ref object __result, object canonicalCard, object owner)
    {
        var toMut = canonicalCard.GetType().GetMethod("ToMutable",
            BindingFlags.Public | BindingFlags.Instance,
            Type.EmptyTypes);
        var mutable = toMut!.Invoke(canonicalCard, null)!;
        // Set Owner on the new card so downstream CardPile.Contains
        // checks don't NRE.
        var ownerProp = mutable.GetType().GetProperty("Owner",
            BindingFlags.Public | BindingFlags.Instance);
        ownerProp?.SetValue(mutable, owner);
        __result = mutable;
        return false;
    }
}

/// Harmony prefix for the generic `NullRunState.CreateCard<T>(Player)`.
/// Reads T from `__originalMethod.GetGenericArguments()[0]`, calls
/// `ModelDb.Card<T>()` to get the canonical, ToMutable's it, assigns
/// Owner, and substitutes the result. Mirrors the real
/// `RunState.CreateCard<T>` body without the run-state bookkeeping.
internal static class NullRunGenericCreateCardBridge
{
    private static MethodInfo? _modelDbCardGen;

    public static bool Prefix(
        System.Reflection.MethodBase __originalMethod,
        object owner,
        ref object __result)
    {
        var t = __originalMethod.GetGenericArguments()[0];
        if (_modelDbCardGen == null)
        {
            var modelDb = t.Assembly.GetType(
                "MegaCrit.Sts2.Core.Models.ModelDb", throwOnError: true)!;
            _modelDbCardGen = modelDb.GetMethods(
                BindingFlags.Public | BindingFlags.Static)
                .First(m => m.Name == "Card" && m.IsGenericMethodDefinition
                    && m.GetParameters().Length == 0);
        }
        var canonical = _modelDbCardGen.MakeGenericMethod(t).Invoke(null, null)!;
        var toMut = canonical.GetType().GetMethod("ToMutable",
            BindingFlags.Public | BindingFlags.Instance, Type.EmptyTypes);
        var mutable = toMut!.Invoke(canonical, null)!;
        var ownerProp = mutable.GetType().GetProperty("Owner",
            BindingFlags.Public | BindingFlags.Instance);
        ownerProp?.SetValue(mutable, owner);
        __result = mutable;
        return false;
    }
}

internal sealed class CombatReflectionBundle
{
    public required ConstructorInfo CombatStateCtor { get; init; }
    public required object UnlockNone { get; init; }
    public required MethodInfo PlayerCreateForNewRun { get; init; }
    public required MethodInfo AddPlayer { get; init; }
    public required MethodInfo CreateCreature { get; init; }
    public required MethodInfo AddCreature { get; init; }
    public required MethodInfo ToMutable { get; init; }
    public required MethodInfo ResetCombatState { get; init; }
    public required MethodInfo PopulateCombatState { get; init; }
    public required MethodInfo GetCharacterByIdMethod { get; init; }
    public required MethodInfo GetMonsterByIdMethod { get; init; }
    public required MethodInfo ModelIdDeserialize { get; init; }
    public required Type CombatSideType { get; init; }
    public required PropertyInfo RoundNumber { get; init; }
    public required PropertyInfo CurrentSide { get; init; }
    public required PropertyInfo Allies { get; init; }
    public required PropertyInfo Enemies { get; init; }
    public required PropertyInfo CreatureName { get; init; }
    public required PropertyInfo CreatureCurrentHp { get; init; }
    public required PropertyInfo CreatureMaxHp { get; init; }
    public required PropertyInfo CreatureBlock { get; init; }
    public required PropertyInfo CreatureIsPlayer { get; init; }
    public required PropertyInfo CreaturePowers { get; init; }
    public required PropertyInfo CreaturePlayer { get; init; }
    public required PropertyInfo PowerAmount { get; init; }
    public required PropertyInfo AbstractModelId { get; init; }
    public required PropertyInfo PlayerCombatState { get; init; }
    public required PropertyInfo PlayerMaxEnergy { get; init; }
    public required PropertyInfo PlayerMasterDeck { get; init; }
    public required PropertyInfo PlayerRelics { get; init; }
    public required PropertyInfo PlayerPotionSlots { get; init; }
    public required PropertyInfo PcsEnergy { get; init; }
    public required PropertyInfo PcsStars { get; init; }
    public required PropertyInfo PcsHand { get; init; }
    public required PropertyInfo PcsDrawPile { get; init; }
    public required PropertyInfo PcsDiscardPile { get; init; }
    public required PropertyInfo PcsExhaustPile { get; init; }
    public required PropertyInfo PcsPlayPile { get; init; }
    public required PropertyInfo PileCards { get; init; }
    public required PropertyInfo CardCurrentUpgradeLevel { get; init; }
    public required PropertyInfo CardEnchantment { get; init; }
    public required PropertyInfo EnchantmentAmountField { get; init; }
    public required MethodInfo CardOnPlay { get; init; }
    public required MethodInfo CardGetResultPileType { get; init; }
    public required ConstructorInfo CardPlayCtor { get; init; }
    public required MethodInfo PileAddInternal { get; init; }
    public required MethodInfo PileRemoveInternal { get; init; }
    public required Type PileTypeEnum { get; init; }
    public required Type CardModelType { get; init; }
    public required MethodInfo CardToMutable { get; init; }
    public required MethodInfo CombatStateAddCardWithOwner { get; init; }
    public required MethodInfo GetCardByIdMethod { get; init; }
    public required MethodInfo GetRelicByIdMethod { get; init; }
    public required MethodInfo RelicToMutable { get; init; }
    public required MethodInfo RelicCmdObtain { get; init; }
    public required MethodInfo BeforeCombatStartMethod { get; init; }
    public required MethodInfo AfterSideTurnStartMethod { get; init; }
    public required ConstructorInfo CombatRoomFromState { get; init; }
    public required MethodInfo AfterRoomEnteredMethod { get; init; }
    public required MethodInfo RunStateCreateForTest { get; init; }
    public required Type GameModeType { get; init; }

    public object GetCharacterById(string id)
    {
        var modelId = ModelIdDeserialize.Invoke(null, new object[] { id })!;
        return GetCharacterByIdMethod.Invoke(null, new object[] { modelId })
            ?? throw new InvalidOperationException(
                $"GetCharacterById returned null for {id}");
    }
    public object GetMonsterById(string id)
    {
        var modelId = ModelIdDeserialize.Invoke(null, new object[] { id })!;
        return GetMonsterByIdMethod.Invoke(null, new object[] { modelId })
            ?? throw new InvalidOperationException(
                $"GetMonsterById returned null for {id}");
    }
    public object GetCardById(string id)
    {
        var modelId = ModelIdDeserialize.Invoke(null, new object[] { id })!;
        return GetCardByIdMethod.Invoke(null, new object[] { modelId })
            ?? throw new InvalidOperationException(
                $"GetCardById returned null for {id}");
    }
    public object GetRelicById(string id)
    {
        var modelId = ModelIdDeserialize.Invoke(null, new object[] { id })!;
        return GetRelicByIdMethod.Invoke(null, new object[] { modelId })
            ?? throw new InvalidOperationException(
                $"GetRelicById returned null for {id}");
    }

    public static CombatReflectionBundle Build(Assembly asm)
    {
        GodotBypass.Apply(asm);

        // Populate ModelDb before any CombatState construction so that
        // Character / Monster / Card / Relic registries are non-empty.
        var modelDb = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb",
            throwOnError: true)!;
        var init = modelDb.GetMethod("Init",
            BindingFlags.Public | BindingFlags.Static)
            ?? throw new InvalidOperationException("ModelDb.Init not found");
        init.Invoke(null, null);
        // InitIds() depends on ModelIdSerializationCache being primed,
        // which itself requires the multiplayer net-id registry to be
        // bootstrapped. Skipping — Init() alone is enough for the
        // ModelDb._contentById lookup (`GetById<T>(id)`). InitIds binds
        // ModelId to instances for net serialization, which our offline
        // dispatcher doesn't need.

        var combatStateType = asm.GetType("MegaCrit.Sts2.Core.Combat.CombatState",
            throwOnError: true)!;
        var combatStateCtor = combatStateType.GetConstructors()
            .First(c => c.GetParameters().Length >= 1);

        var playerType = asm.GetType("MegaCrit.Sts2.Core.Entities.Players.Player",
            throwOnError: true)!;
        var characterModelType = asm.GetType(
            "MegaCrit.Sts2.Core.Models.CharacterModel",
            throwOnError: true)!;
        var unlockStateType = asm.GetType("MegaCrit.Sts2.Core.Unlocks.UnlockState",
            throwOnError: true)!;
        // UnlockState has a static `none` field with an empty unlock set —
        // perfect for the mock combat scaffold.
        var unlockNoneField = unlockStateType.GetField("none",
            BindingFlags.Public | BindingFlags.Static)
            ?? throw new InvalidOperationException("UnlockState.none static field not found");
        var unlockNone = unlockNoneField.GetValue(null)
            ?? throw new InvalidOperationException("UnlockState.none was null");
        // Player.CreateForNewRun(CharacterModel, UnlockState, ulong)
        var playerCreate = playerType.GetMethods(
            BindingFlags.Public | BindingFlags.Static)
            .First(m => m.Name == "CreateForNewRun"
                && m.GetParameters().Length == 3
                && !m.IsGenericMethod
                && m.GetParameters()[0].ParameterType == characterModelType);

        var addPlayer = combatStateType.GetMethod("AddPlayer",
            BindingFlags.Public | BindingFlags.Instance)
            ?? throw new InvalidOperationException("CombatState.AddPlayer not found");
        var createCreature = combatStateType.GetMethod("CreateCreature",
            BindingFlags.Public | BindingFlags.Instance)
            ?? throw new InvalidOperationException("CombatState.CreateCreature not found");
        var addCreature = combatStateType.GetMethod("AddCreature",
            BindingFlags.Public | BindingFlags.Instance,
            new[] { asm.GetType("MegaCrit.Sts2.Core.Entities.Creatures.Creature",
                throwOnError: true)! })
            ?? throw new InvalidOperationException("CombatState.AddCreature not found");

        var monsterModelType = asm.GetType("MegaCrit.Sts2.Core.Models.MonsterModel",
            throwOnError: true)!;
        var toMutable = monsterModelType.GetMethod("ToMutable",
            BindingFlags.Public | BindingFlags.Instance,
            Type.EmptyTypes)
            ?? throw new InvalidOperationException("MonsterModel.ToMutable not found");
        var resetCombatState = playerType.GetMethod("ResetCombatState",
            BindingFlags.Public | BindingFlags.Instance)
            ?? throw new InvalidOperationException("Player.ResetCombatState not found");
        var populateCombatState = playerType.GetMethod("PopulateCombatState",
            BindingFlags.Public | BindingFlags.Instance)
            ?? throw new InvalidOperationException("Player.PopulateCombatState not found");

        // ModelDb.GetById<T>(ModelId id). We need ModelId.Deserialize(string)
        // to bridge from the string id ("CHARACTER.IRONCLAD") to ModelId.
        var modelIdType = asm.GetType("MegaCrit.Sts2.Core.Models.ModelId",
            throwOnError: true)!;
        var modelIdDeserialize = modelIdType.GetMethod("Deserialize",
            BindingFlags.Public | BindingFlags.Static,
            new[] { typeof(string) })
            ?? throw new InvalidOperationException("ModelId.Deserialize(string) not found");
        var getByIdGen = modelDb.GetMethods(
            BindingFlags.Public | BindingFlags.Static)
            .First(m => m.Name == "GetById" && m.IsGenericMethod
                && m.GetParameters().Length == 1
                && m.GetParameters()[0].ParameterType == modelIdType);
        var getCharacterById = getByIdGen.MakeGenericMethod(characterModelType);
        var getMonsterById = getByIdGen.MakeGenericMethod(monsterModelType);

        var combatSideType = asm.GetType("MegaCrit.Sts2.Core.Combat.CombatSide",
            throwOnError: true)!;

        var roundNumber = combatStateType.GetProperty("RoundNumber")
            ?? throw new InvalidOperationException("RoundNumber not found");
        var currentSide = combatStateType.GetProperty("CurrentSide")
            ?? throw new InvalidOperationException("CurrentSide not found");
        var allies = combatStateType.GetProperty("Allies")
            ?? throw new InvalidOperationException("Allies not found");
        var enemies = combatStateType.GetProperty("Enemies")
            ?? throw new InvalidOperationException("Enemies not found");

        var creatureType = asm.GetType("MegaCrit.Sts2.Core.Entities.Creatures.Creature",
            throwOnError: true)!;
        var creatureName = creatureType.GetProperty("Name")
            ?? throw new InvalidOperationException("Creature.Name not found");
        var creatureCurrentHp = creatureType.GetProperty("CurrentHp")
            ?? throw new InvalidOperationException("Creature.CurrentHp not found");
        var creatureMaxHp = creatureType.GetProperty("MaxHp")
            ?? throw new InvalidOperationException("Creature.MaxHp not found");
        var creatureBlock = creatureType.GetProperty("Block")
            ?? throw new InvalidOperationException("Creature.Block not found");
        var creatureIsPlayer = creatureType.GetProperty("IsPlayer")
            ?? throw new InvalidOperationException("Creature.IsPlayer not found");
        var creaturePowers = creatureType.GetProperty("Powers")
            ?? throw new InvalidOperationException("Creature.Powers not found");
        var creaturePlayer = creatureType.GetProperty("Player")
            ?? throw new InvalidOperationException("Creature.Player not found");

        var powerType = asm.GetType("MegaCrit.Sts2.Core.Models.PowerModel",
            throwOnError: true)!;
        var powerAmount = powerType.GetProperty("Amount")
            ?? throw new InvalidOperationException("PowerModel.Amount not found");
        var abstractModelType = asm.GetType("MegaCrit.Sts2.Core.Models.AbstractModel",
            throwOnError: true)!;
        var abstractModelId = abstractModelType.GetProperty("Id")
            ?? throw new InvalidOperationException("AbstractModel.Id not found");

        var pcsType = asm.GetType(
            "MegaCrit.Sts2.Core.Entities.Players.PlayerCombatState",
            throwOnError: true)!;
        var playerCombatStateProp = playerType.GetProperty("PlayerCombatState")
            ?? throw new InvalidOperationException("Player.PlayerCombatState not found");
        var playerMaxEnergy = playerType.GetProperty("MaxEnergy")
            ?? throw new InvalidOperationException("Player.MaxEnergy not found");
        var playerMasterDeck = playerType.GetProperty("Deck")
            ?? throw new InvalidOperationException("Player.Deck not found");
        var playerRelics = playerType.GetProperty("Relics")
            ?? throw new InvalidOperationException("Player.Relics not found");
        var playerPotionSlots = playerType.GetProperty("PotionSlots")
            ?? throw new InvalidOperationException("Player.PotionSlots not found");

        var pcsEnergy = pcsType.GetProperty("Energy")
            ?? throw new InvalidOperationException("Pcs.Energy not found");
        var pcsStars = pcsType.GetProperty("Stars")
            ?? throw new InvalidOperationException("Pcs.Stars not found");
        var pcsHand = pcsType.GetProperty("Hand")
            ?? throw new InvalidOperationException("Pcs.Hand not found");
        var pcsDraw = pcsType.GetProperty("DrawPile")
            ?? throw new InvalidOperationException("Pcs.DrawPile not found");
        var pcsDiscard = pcsType.GetProperty("DiscardPile")
            ?? throw new InvalidOperationException("Pcs.DiscardPile not found");
        var pcsExhaust = pcsType.GetProperty("ExhaustPile")
            ?? throw new InvalidOperationException("Pcs.ExhaustPile not found");
        var pcsPlay = pcsType.GetProperty("PlayPile")
            ?? throw new InvalidOperationException("Pcs.PlayPile not found");

        var pileType = asm.GetType("MegaCrit.Sts2.Core.Entities.Cards.CardPile",
            throwOnError: true)!;
        var pileCards = pileType.GetProperty("Cards")
            ?? throw new InvalidOperationException("CardPile.Cards not found");

        var cardType = asm.GetType("MegaCrit.Sts2.Core.Models.CardModel",
            throwOnError: true)!;
        var cardUpgrade = cardType.GetProperty("CurrentUpgradeLevel")
            ?? throw new InvalidOperationException("CardModel.CurrentUpgradeLevel not found");
        var cardEnchantment = cardType.GetProperty("Enchantment")
            ?? throw new InvalidOperationException("CardModel.Enchantment not found");
        var enchantmentType = asm.GetType(
            "MegaCrit.Sts2.Core.Models.EnchantmentModel",
            throwOnError: true)!;
        var enchantmentAmount = enchantmentType.GetProperty("Amount")
            ?? throw new InvalidOperationException("Enchantment.Amount not found");

        var playerChoiceContextType = asm.GetType(
            "MegaCrit.Sts2.Core.GameActions.Multiplayer.PlayerChoiceContext",
            throwOnError: true)!;
        var cardPlayType = asm.GetType(
            "MegaCrit.Sts2.Core.Entities.Cards.CardPlay",
            throwOnError: true)!;
        var cardOnPlay = cardType.GetMethod("OnPlay",
            BindingFlags.NonPublic | BindingFlags.Instance,
            new[] { playerChoiceContextType, cardPlayType })
            ?? throw new InvalidOperationException("CardModel.OnPlay not found");
        var cardGetResultPileType = cardType.GetMethod("GetResultPileType",
            BindingFlags.NonPublic | BindingFlags.Instance,
            Type.EmptyTypes)
            ?? throw new InvalidOperationException("CardModel.GetResultPileType not found");
        var cardPlayCtor = cardPlayType.GetConstructor(Type.EmptyTypes)
            ?? throw new InvalidOperationException("CardPlay() ctor not found");

        var pileAddInternal = pileType.GetMethod("AddInternal",
            BindingFlags.Public | BindingFlags.Instance)
            ?? throw new InvalidOperationException("CardPile.AddInternal not found");
        var pileRemoveInternal = pileType.GetMethod("RemoveInternal",
            BindingFlags.Public | BindingFlags.Instance)
            ?? throw new InvalidOperationException("CardPile.RemoveInternal not found");

        var pileTypeEnum = asm.GetType("MegaCrit.Sts2.Core.Entities.Cards.PileType",
            throwOnError: true)!;

        var cardToMutable = cardType.GetMethod("ToMutable",
            BindingFlags.Public | BindingFlags.Instance,
            Type.EmptyTypes)
            ?? throw new InvalidOperationException("CardModel.ToMutable not found");
        // CombatState.AddCard(CardModel, Player) — the 2-arg overload
        // that sets Owner.
        var addCardWithOwner = combatStateType.GetMethod("AddCard",
            BindingFlags.Public | BindingFlags.Instance,
            new[] { cardType, playerType })
            ?? throw new InvalidOperationException("CombatState.AddCard(card, owner) not found");
        var getCardById = getByIdGen.MakeGenericMethod(cardType);

        // Relic reflection: ModelDb.GetById<RelicModel>(id) +
        // RelicCmd.Obtain(relic, player, -1) + AfterObtained.
        var relicModelType = asm.GetType(
            "MegaCrit.Sts2.Core.Models.RelicModel", throwOnError: true)!;
        var getRelicById = getByIdGen.MakeGenericMethod(relicModelType);
        var relicToMutable = relicModelType.GetMethod("ToMutable",
            BindingFlags.Public | BindingFlags.Instance, Type.EmptyTypes)
            ?? throw new InvalidOperationException("RelicModel.ToMutable not found");
        var relicCmdType = asm.GetType(
            "MegaCrit.Sts2.Core.Commands.RelicCmd", throwOnError: true)!;
        // RelicCmd.Obtain(RelicModel, Player, int) — the 3-arg overload.
        var relicObtain = relicCmdType.GetMethods(
            BindingFlags.Public | BindingFlags.Static)
            .First(m => m.Name == "Obtain"
                && !m.IsGenericMethod
                && m.GetParameters().Length == 3
                && m.GetParameters()[0].ParameterType == relicModelType
                && m.GetParameters()[1].ParameterType == playerType);
        // BeforeCombatStart is declared virtual on AbstractModel and lives
        // in the model class. Reflection by name on AbstractModel covers
        // every subclass via virtual dispatch.
        var beforeCombatStart = abstractModelType.GetMethod(
            "BeforeCombatStart",
            BindingFlags.Public | BindingFlags.Instance,
            Type.EmptyTypes)
            ?? throw new InvalidOperationException("AbstractModel.BeforeCombatStart not found");
        var afterSideTurnStart = abstractModelType.GetMethod(
            "AfterSideTurnStart",
            BindingFlags.Public | BindingFlags.Instance,
            new[] { combatSideType, combatStateType })
            ?? throw new InvalidOperationException("AbstractModel.AfterSideTurnStart not found");
        // CombatRoom(CombatState) ctor — synthesizes the AbstractRoom
        // arg that AfterRoomEntered relics inspect via `room is CombatRoom`.
        var combatRoomType = asm.GetType(
            "MegaCrit.Sts2.Core.Rooms.CombatRoom", throwOnError: true)!;
        var combatRoomFromState = combatRoomType.GetConstructor(
            new[] { combatStateType })
            ?? throw new InvalidOperationException("CombatRoom(CombatState) not found");
        var abstractRoomType = asm.GetType(
            "MegaCrit.Sts2.Core.Rooms.AbstractRoom", throwOnError: true)!;
        var afterRoomEntered = abstractModelType.GetMethod(
            "AfterRoomEntered",
            BindingFlags.Public | BindingFlags.Instance,
            new[] { abstractRoomType })
            ?? throw new InvalidOperationException("AbstractModel.AfterRoomEntered not found");
        // RunState.CreateForTest — upgrades NullRunState → real RunState.
        // Signature: (players, acts, modifiers, gameMode, ascensionLevel, seed)
        // with all params nullable / defaulted. We call with our existing
        // player list so the RunState back-references the player and
        // registers its deck cards.
        var runStateType = asm.GetType("MegaCrit.Sts2.Core.Runs.RunState",
            throwOnError: true)!;
        var runStateCreateForTest = runStateType.GetMethod("CreateForTest",
            BindingFlags.Public | BindingFlags.Static)
            ?? throw new InvalidOperationException("RunState.CreateForTest not found");
        var gameModeType = asm.GetType("MegaCrit.Sts2.Core.Runs.GameMode",
            throwOnError: true)!;

        return new CombatReflectionBundle
        {
            CombatStateCtor = combatStateCtor,
            UnlockNone = unlockNone,
            ResetCombatState = resetCombatState,
            PopulateCombatState = populateCombatState,
            PlayerCreateForNewRun = playerCreate,
            AddPlayer = addPlayer,
            CreateCreature = createCreature,
            AddCreature = addCreature,
            ToMutable = toMutable,
            GetCharacterByIdMethod = getCharacterById,
            GetMonsterByIdMethod = getMonsterById,
            ModelIdDeserialize = modelIdDeserialize,
            CombatSideType = combatSideType,
            RoundNumber = roundNumber,
            CurrentSide = currentSide,
            Allies = allies,
            Enemies = enemies,
            CreatureName = creatureName,
            CreatureCurrentHp = creatureCurrentHp,
            CreatureMaxHp = creatureMaxHp,
            CreatureBlock = creatureBlock,
            CreatureIsPlayer = creatureIsPlayer,
            CreaturePowers = creaturePowers,
            CreaturePlayer = creaturePlayer,
            PowerAmount = powerAmount,
            AbstractModelId = abstractModelId,
            PlayerCombatState = playerCombatStateProp,
            PlayerMaxEnergy = playerMaxEnergy,
            PlayerMasterDeck = playerMasterDeck,
            PlayerRelics = playerRelics,
            PlayerPotionSlots = playerPotionSlots,
            PcsEnergy = pcsEnergy,
            PcsStars = pcsStars,
            PcsHand = pcsHand,
            PcsDrawPile = pcsDraw,
            PcsDiscardPile = pcsDiscard,
            PcsExhaustPile = pcsExhaust,
            PcsPlayPile = pcsPlay,
            PileCards = pileCards,
            CardCurrentUpgradeLevel = cardUpgrade,
            CardEnchantment = cardEnchantment,
            EnchantmentAmountField = enchantmentAmount,
            CardOnPlay = cardOnPlay,
            CardGetResultPileType = cardGetResultPileType,
            CardPlayCtor = cardPlayCtor,
            PileAddInternal = pileAddInternal,
            PileRemoveInternal = pileRemoveInternal,
            PileTypeEnum = pileTypeEnum,
            CardModelType = cardType,
            CardToMutable = cardToMutable,
            CombatStateAddCardWithOwner = addCardWithOwner,
            GetCardByIdMethod = getCardById,
            GetRelicByIdMethod = getRelicById,
            RelicToMutable = relicToMutable,
            RelicCmdObtain = relicObtain,
            BeforeCombatStartMethod = beforeCombatStart,
            AfterSideTurnStartMethod = afterSideTurnStart,
            CombatRoomFromState = combatRoomFromState,
            AfterRoomEnteredMethod = afterRoomEntered,
            RunStateCreateForTest = runStateCreateForTest,
            GameModeType = gameModeType,
        };
    }
}

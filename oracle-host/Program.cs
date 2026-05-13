// Oracle host. Reflectively loads the shipping sts2.dll and exposes its
// functions over stdio JSON-RPC. Used by sts2-sim-oracle-tests to assert the
// Rust port matches real-game behavior bit-exactly.
//
// Current state: skeleton dispatch loop accepting "ping". Reflective loading
// of sts2.dll is implemented incrementally as Rust modules are ported.

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
            Console.Error.WriteLine("Set STS2_GAME_DIR or update DefaultGameDir.");
            return 2;
        }

        var dllPath = Path.Combine(gameDir, "sts2.dll");
        if (!File.Exists(dllPath))
        {
            Console.Error.WriteLine($"oracle-host: sts2.dll not found at {dllPath}");
            return 2;
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
                response = method switch
                {
                    "ping" => new JsonObject
                    {
                        ["result"] = "pong",
                        ["game_dir"] = gameDir,
                    },
                    _ => new JsonObject { ["error"] = $"unknown method: {method}" },
                };
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

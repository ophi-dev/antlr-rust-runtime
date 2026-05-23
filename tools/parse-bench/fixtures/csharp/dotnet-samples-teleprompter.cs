// Reference pattern: dotnet/samples console teleprompter.
// Source: https://github.com/dotnet/samples/blob/main/csharp/getting-started/console-teleprompter/Program.cs
// Upstream license: MIT. This fixture is a compact benchmark excerpt.

using System;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace TeleprompterConsole
{
    public class Program
    {
        public static async Task Main(string[] args)
        {
            await RunTeleprompter();
        }

        private static async Task RunTeleprompter()
        {
            var config = new TelePrompterConfig();
            var displayTask = ShowTeleprompter(config);
            var speedTask = GetInput(config);
            await Task.WhenAny(displayTask, speedTask);
        }

        private static async Task ShowTeleprompter(TelePrompterConfig config)
        {
            var words = ReadFrom("sampleQuotes.txt");
            foreach (var word in words)
            {
                Console.Write(word);
                if (!string.IsNullOrWhiteSpace(word))
                {
                    await Task.Delay(config.DelayInMilliseconds);
                }
            }
        }

        private static IEnumerable<string> ReadFrom(string file)
        {
            return new[] { "hello", "from", file };
        }
    }
}

// Reference pattern: Newtonsoft.Json JsonConvert utility class.
// Source: https://github.com/JamesNK/Newtonsoft.Json/blob/master/Src/Newtonsoft.Json/JsonConvert.cs
// Upstream license: MIT. This fixture is a compact benchmark excerpt.

using System;
using System.Globalization;

namespace Newtonsoft.Json
{
    public static class JsonConvert
    {
        public static readonly string True = "true";
        public static readonly string False = "false";
        public static readonly string Null = "null";

        public static string ToString(bool value)
        {
            return value ? True : False;
        }

        public static string ToString(DateTime value)
        {
            return value.ToString("o", CultureInfo.InvariantCulture);
        }

        public static string SerializeObject(object value, params JsonConverter[] converters)
        {
            if (value == null)
            {
                return Null;
            }

            var writer = new JsonTextWriter();
            foreach (JsonConverter converter in converters)
            {
                writer.Converters.Add(converter);
            }

            writer.WriteValue(value);
            return writer.ToString();
        }
    }
}

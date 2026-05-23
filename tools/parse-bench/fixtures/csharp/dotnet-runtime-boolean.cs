// Reference pattern: dotnet/runtime System.Boolean implementation.
// Source: https://github.com/dotnet/runtime/blob/main/src/libraries/System.Private.CoreLib/src/System/Boolean.cs
// Upstream license: MIT. This fixture is a compact benchmark excerpt.

using System;
using System.Runtime.CompilerServices;

namespace System
{
    [Serializable]
    public struct Boolean : IComparable, IComparable<bool>, IEquatable<bool>
    {
        private readonly bool m_value;

        public int CompareTo(object obj)
        {
            if (obj == null) return 1;
            if (!(obj is bool)) throw new ArgumentException("Object must be boolean");
            return CompareTo((bool)obj);
        }

        [MethodImpl(MethodImplOptions.AggressiveInlining)]
        public int CompareTo(bool value)
        {
            if (m_value == value) return 0;
            return m_value ? 1 : -1;
        }

        public bool Equals(bool obj)
        {
            return m_value == obj;
        }

        public override string ToString()
        {
            return m_value ? "True" : "False";
        }
    }
}

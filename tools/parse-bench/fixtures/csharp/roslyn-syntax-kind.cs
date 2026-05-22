// Reference pattern: Roslyn SyntaxKind enum.
// Source: https://github.com/dotnet/roslyn/blob/main/src/Compilers/CSharp/Portable/Syntax/SyntaxKind.cs
// Upstream license: MIT. This fixture is a compact benchmark excerpt.

using System.Diagnostics.CodeAnalysis;

namespace Microsoft.CodeAnalysis.CSharp
{
#pragma warning disable CA1200
    public enum SyntaxKind : ushort
    {
        None = 0,
        List = 1,

        /// <summary>Represents <c>~</c> token.</summary>
        TildeToken = 8193,

        /// <summary>Represents <c>!</c> token.</summary>
        ExclamationToken = 8194,

        /// <summary>Represents <c>$</c> token.</summary>
        DollarToken = 8195,

        /// <summary>Represents <c>identifier</c> token.</summary>
        IdentifierToken = 8508,

        EndOfFileToken = 8511,
        CompilationUnit = 8840,
        NamespaceDeclaration = 8841,
        ClassDeclaration = 8855,
        MethodDeclaration = 8875
    }
#pragma warning restore CA1200
}

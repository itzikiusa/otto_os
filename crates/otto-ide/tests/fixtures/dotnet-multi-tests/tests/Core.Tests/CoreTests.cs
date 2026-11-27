using Calc;
using Xunit;

namespace Core.Tests;

public class CoreTests
{
    [Fact]
    public void AddsSmallNumbers() => Assert.Equal(3, Calculator.Add(1, 2));

    [Fact]
    public void AddsNegatives() => Assert.Equal(-3, Calculator.Add(-1, -2));

    [Fact]
    public void AddsZero() => Assert.Equal(5, Calculator.Add(5, 0));
}

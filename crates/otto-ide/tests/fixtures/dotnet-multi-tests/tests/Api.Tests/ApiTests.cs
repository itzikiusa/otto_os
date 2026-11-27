using Calc;
using NUnit.Framework;

namespace Api.Tests;

public class ApiTests
{
    [Test]
    public void AddsForTheApi() => Assert.That(Calculator.Add(20, 22), Is.EqualTo(42));

    [Test]
    public void FailsOnlyOnNet11()
    {
        var expected = 4;
#if NET11_0_OR_GREATER
        expected = 5;
#endif
        Assert.That(Calculator.Add(2, 2), Is.EqualTo(expected));
    }
}

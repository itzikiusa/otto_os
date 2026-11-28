using Calc;
using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Mtp.Tests;

[TestClass]
public class MtpTests
{
    [TestMethod]
    public void AddsOnTheTestingPlatform() => Assert.AreEqual(7, Calculator.Add(3, 4));
}

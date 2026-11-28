using System;
using System.Threading;

// Stands in for the Unity engine: loads Assembly-CSharp and calls the script every 100 ms.
public static class Host
{
    public static void Main()
    {
        for (int frame = 0; ; frame++)
        {
            Player.Update(frame);
            Thread.Sleep(100);
        }
    }
}

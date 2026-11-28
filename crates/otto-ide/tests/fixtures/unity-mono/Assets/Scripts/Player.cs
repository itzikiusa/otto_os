using System;
using System.Collections.Generic;

// A Unity-style script: the host calls Update once per frame, as the engine does.
public class Player
{
    public static int speed = 3;
    static readonly List<int> history = new List<int>();

    public static void Update(int frame)
    {
        string name = "player-" + frame;
        history.Add(frame);
        int doubled = Helper.Compute(frame);
        if (frame % 10 == 8)
        {
            try { throw new InvalidOperationException("frame " + frame); }
            catch (InvalidOperationException) { }
        }
        Console.WriteLine("frame=" + frame + " speed=" + speed + " name=" + name + " doubled=" + doubled + " n=" + history.Count);
    }
}

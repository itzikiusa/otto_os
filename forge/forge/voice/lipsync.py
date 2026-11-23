"""Align English dialogue to the final WAV and map its measured phones to A–H/X."""

import math
import re
import unicodedata
import wave

# Rhubarb shape vocabulary: closure, consonant, medium/wide vowel, rounded,
# pursed lips, lower lip against teeth, tongue up, and rest.
SHAPES = {
    phone: shape
    for shape, phones in {
        "A": "P B M",
        "B": "T D K G N NG S Z SH ZH CH JH TH DH Y HH",
        "C": "IH IY EH EY AE",
        "D": "AA AH AX AY AW",
        "E": "AO OW OY ER R",
        "F": "UH UW W",
        "G": "F V",
        "H": "L",
        "X": "SIL +SPN+ +NSN+",
    }.items()
    for phone in phones.split()
}


def align(wav, text, duration):
    import numpy as np
    from num2words import num2words
    from pocketsphinx import Decoder
    from scipy.signal import resample_poly

    text = unicodedata.normalize("NFKC", text).lower().replace("’", "'")
    text = re.sub(r"\d+(?:\.\d+)?", lambda m: num2words(m.group()), text)
    words = re.findall(r"[a-z]+(?:'[a-z]+)*", text)
    decoder = Decoder(lm=None, samprate=16000, frate=100, loglevel="ERROR")
    for word in dict.fromkeys(words):
        if not decoder.lookup_word(word):
            # Give names outside CMUdict an English pronunciation using the Kokoro frontend.
            from types import SimpleNamespace

            from misaki.espeak import EspeakFallback

            phones, _ = EspeakFallback(british=False)(SimpleNamespace(text=word))
            ipa = "p:P b:B t:T d:D k:K ɡ:G g:G f:F v:V θ:TH ð:DH s:S z:Z ʃ:SH ʒ:ZH ʧ:CH ʤ:JH h:HH m:M n:N ŋ:NG l:L ɹ:R j:Y w:W ɪ:IH i:IY ɛ:EH æ:AE ə:AH ᵊ:AH ʌ:AH ɑ:AA a:AA ɒ:AO ɔ:AO ʊ:UH u:UW ɜ:ER I:AY A:EY O:OW Q:OW W:AW Y:OY T:T"
            mapping = dict(pair.split(":") for pair in ipa.split())
            pronunciation = " ".join(mapping[c] for c in phones or "" if c in mapping)
            if not pronunciation:
                raise ValueError(
                    f'Cannot align "{word}". Spell the pronunciation in English.'
                )
            decoder.add_word(word, pronunciation, False)
    if not words:
        raise ValueError("No English words for lip sync")
    with wave.open(str(wav), "rb") as stream:
        rate = stream.getframerate()
        signal = np.frombuffer(
            stream.readframes(stream.getnframes()), dtype="<i2"
        ).astype(float)
    divisor = math.gcd(rate, 16000)
    pcm = (
        resample_poly(signal, 16000 // divisor, rate // divisor)
        .clip(-32768, 32767)
        .astype("<i2")
        .tobytes()
    )
    decoder.set_align_text(" ".join(words))

    def decode():
        decoder.start_utt()
        decoder.process_raw(pcm, full_utt=True)
        decoder.end_utt()

    decode()
    decoder.set_alignment()
    decode()
    cues = []

    def append(start, end, value):
        if end <= start:
            return
        if cues and cues[-1]["value"] == value:
            cues[-1]["end"] = end
        else:
            cues.append({"start": start, "end": end, "value": value})

    cursor = 0.0
    for word in decoder.get_alignment():
        for phone in word:
            start = min(duration, max(cursor, phone.start / 100))
            end = min(duration, (phone.start + phone.duration) / 100)
            append(cursor, start, "X")
            append(start, end, SHAPES[phone.name])
            cursor = max(start, end)
    if not cues:
        raise ValueError("Unable to align the dialogue to the audio")
    append(cursor, duration, "X")
    return {"duration_sec": duration, "cues": cues}

"""Hammerfest source names -> obfuscated SWF names.

The table comes from `eternalfest/game-types` (see vendor/NOTICE.md). It saves
all the dynamic reverse work: no need to look for `currentId` by memory
differences, we know its exact key.

    >>> obf("currentId")
    '-BBEO'
    >>> clear("70dik")
    'realScores'

An identifier missing from the table is one the obfuscator did not rename
(standard AS2 API: `duration`, `length`...). It keeps its clear name, so
identity is the right answer and not an error.
"""
import json
import os

_HERE = os.path.dirname(os.path.abspath(__file__))
MAP_PATH = os.path.join(_HERE, os.pardir, "vendor", "hf.map.json")

with open(MAP_PATH, encoding="utf-8") as _f:
    CLEAR_TO_OBF = json.load(_f)

OBF_TO_CLEAR = {}
for _c, _o in CLEAR_TO_OBF.items():
    OBF_TO_CLEAR.setdefault(_o, _c)


def obf(name):
    """The property name as it appears in the SWF."""
    return CLEAR_TO_OBF.get(name, name)


def clear(name):
    """The source name for a key read from memory, or the key itself."""
    return OBF_TO_CLEAR.get(name, name)


def is_renamed(name):
    return name in CLEAR_TO_OBF

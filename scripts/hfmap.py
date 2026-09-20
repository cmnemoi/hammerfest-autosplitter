"""Noms du source Hammerfest -> noms obfusques du SWF.

La table vient de `eternalfest/game-types` (voir vendor/NOTICE.md). Elle evite
tout le reverse dynamique : plus besoin de chercher `currentId` par differences
memoire, on connait sa clef exacte.

    >>> obf("currentId")
    '-BBEO'
    >>> clear("70dik")
    'realScores'

Un identifiant absent de la table est un identifiant que l'obfuscateur n'a pas
renomme (API AS2 standard : `duration`, `length`...). Il garde son nom en clair,
donc l'identite est la bonne reponse et non une erreur.
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
    """Nom de propriete tel qu'il apparait dans le SWF."""
    return CLEAR_TO_OBF.get(name, name)


def clear(name):
    """Nom du source pour une clef lue en memoire, ou la clef elle-meme."""
    return OBF_TO_CLEAR.get(name, name)


def is_renamed(name):
    return name in CLEAR_TO_OBF

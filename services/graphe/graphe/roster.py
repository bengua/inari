"""The agent roster: one YAML file says who each agent is — a name, a harness,
a model, a purpose, a write scope. ADW call sites name an agent, never a model.
"""

import yaml


class RosterError(ValueError):
    pass


class Roster:
    def __init__(self, defaults, agents):
        self.defaults = defaults
        self.agents = agents  # name -> entry

    @classmethod
    def load(cls, path):
        with open(path, encoding="utf-8") as f:
            doc = yaml.safe_load(f) or {}
        defaults = doc.get("defaults", {})
        agents = {}
        for entry in doc.get("agents", []):
            name = entry.get("name", "")
            if not name:
                raise RosterError("a roster entry has no name")
            if name in agents:
                raise RosterError(f"duplicate agent {name!r}")
            if entry.get("model", "") == name:
                raise RosterError(
                    f"agent {name!r}: the roster names agents, not models")
            merged = {**{k: v for k, v in defaults.items() if k in (
                "harness", "model")}, **entry}
            agents[name] = merged
        if not agents:
            raise RosterError("the roster names no agent")
        return cls(defaults, agents)

    def require(self, names):
        """A missing agent fails before anything spawns."""
        missing = [n for n in names if n not in self.agents]
        if missing:
            raise RosterError(f"roster misses: {', '.join(missing)}")

    def agent(self, name):
        try:
            return self.agents[name]
        except KeyError:
            raise RosterError(f"unknown agent {name!r}") from None

    def write_scope(self, name):
        return list(self.agent(name).get("writes", []))

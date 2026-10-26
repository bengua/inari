"""Exceptions shared across the package."""


class InariError(Exception):
    """Base class for every error this package raises on purpose."""


class IntegrityError(InariError):
    """A digest, a hash chain or a capsule layout does not match."""


class ConflictError(InariError):
    """Two different events claim the same identity (equivocation)."""


class TransportError(InariError):
    """The link to another node failed; the operation may or may not have landed."""


class IllegalTransition(InariError):
    """The state machine was driven with an input it does not accept."""

import hashlib
import secrets
from collections.abc import Awaitable, Callable, Sequence
from datetime import UTC, datetime, timedelta
from typing import Annotated, Final, TypeAlias
from urllib.parse import urlsplit

import jwt
from fastapi import APIRouter, Depends, HTTPException, Request, Response
from pydantic import SecretStr, TypeAdapter, ValidationError

from litellm_lens.context import current_runtime
from litellm_lens.identity import AllRows, Identity, OwnedRows, ReadScope, Role
from litellm_lens.models import Record

router: Final = APIRouter(prefix="/auth", tags=["Authentication"])
COOKIE: Final = "lens_session"
SESSION_LIFETIME: Final = timedelta(hours=8)


class SessionRequest(Record):
    token: SecretStr


class SessionView(Record):
    user_id: str
    user_role: Role


class GatewayClaims(Record):
    iss: str
    aud: str
    sub: str
    iat: int
    exp: int
    identity: Identity


def token_hash(token: str) -> str:
    return hashlib.sha256(token.encode()).hexdigest()


def local_admin() -> Identity:
    return Identity(user_id="lens-admin", user_role=Role.PROXY_ADMIN)


def delegated_identity(token: str) -> Identity:
    secret: Final = current_runtime().gateway_secret
    if secret is None:
        raise HTTPException(401, "Invalid Lens credential")
    try:
        claims: Final = GatewayClaims.model_validate(
            jwt.decode(
                token,
                secret.get_secret_value(),
                algorithms=["HS256"],
                issuer="litellm",
                audience="litellm-lens",
                options={"require": ["iss", "aud", "sub", "iat", "exp"]},
            )
        )
    except (jwt.InvalidTokenError, ValidationError) as error:
        raise HTTPException(401, "Invalid or expired gateway identity") from error
    if claims.exp - claims.iat > 60 or claims.sub != (claims.identity.user_id or claims.identity.token):
        raise HTTPException(401, "Invalid gateway identity scope")
    return claims.identity


def check_cookie_origin(request: Request) -> None:
    if request.method in ("GET", "HEAD", "OPTIONS"):
        return
    origin: Final = request.headers.get("origin")
    configured: Final = urlsplit(current_runtime().public_url)
    expected: Final = f"{configured.scheme}://{configured.netloc}"
    if origin != expected:
        raise HTTPException(403, "Lens session requests must come from the Lens origin")


async def user_api_key_auth(request: Request) -> Identity:
    authorization: Final = request.headers.get("authorization", "")
    if authorization:
        scheme, _, token = authorization.partition(" ")
        if scheme.lower() != "bearer" or not token:
            raise HTTPException(401, "Use a Lens bearer credential")
        if secrets.compare_digest(token, current_runtime().admin_token.get_secret_value()):
            return local_admin()
        return delegated_identity(token)
    session: Final = request.cookies.get(COOKIE)
    if session is None:
        raise HTTPException(401, "Sign in to Lens")
    check_cookie_origin(request)
    rows: Final = TypeAdapter(tuple[dict[str, str], ...]).validate_python(
        await current_runtime().database.query_raw(
            'SELECT id FROM "LensSession" WHERE id=$1 AND expires_at > CURRENT_TIMESTAMP', token_hash(session)
        )
    )
    if not rows:
        raise HTTPException(401, "Lens session has expired")
    return local_admin()


@router.post("/session", response_model=SessionView)
async def sign_in(body: SessionRequest, response: Response) -> SessionView:
    if not secrets.compare_digest(body.token.get_secret_value(), current_runtime().admin_token.get_secret_value()):
        raise HTTPException(401, "Invalid Lens setup token")
    session: Final = secrets.token_urlsafe(48)
    expires: Final = datetime.now(UTC) + SESSION_LIFETIME
    await current_runtime().database.execute_raw(
        'INSERT INTO "LensSession" (id, expires_at) VALUES ($1, $2)', token_hash(session), expires
    )
    response.set_cookie(
        COOKIE,
        session,
        max_age=int(SESSION_LIFETIME.total_seconds()),
        httponly=True,
        secure=current_runtime().public_url.startswith("https://"),
        samesite="strict",
    )
    return SessionView(user_id="lens-admin", user_role=Role.PROXY_ADMIN)


@router.get("/session", response_model=SessionView)
async def session_info(auth: Annotated[Identity, Depends(user_api_key_auth)]) -> SessionView:
    return SessionView(user_id=auth.user_id or "", user_role=auth.user_role)


@router.delete("/session", status_code=204)
async def sign_out(request: Request, auth: Annotated[Identity, Depends(user_api_key_auth)]) -> Response:
    session: Final = request.cookies.get(COOKIE)
    if session is not None:
        await current_runtime().database.execute_raw('DELETE FROM "LensSession" WHERE id=$1', token_hash(session))
    response: Final = Response(status_code=204)
    response.delete_cookie(COOKIE)
    return response


async def permitted_log_teams(auth: Identity) -> tuple[str, ...]:
    return auth.log_team_ids


def provide_log_team_lookup() -> Callable[[Identity], Awaitable[tuple[str, ...]]]:
    return permitted_log_teams


LogTeamLookupDependency: TypeAlias = Annotated[
    Callable[[Identity], Awaitable[tuple[str, ...]]], Depends(provide_log_team_lookup)
]


async def resolve_trace_read_scope(
    auth: Identity, permitted_team_lookup: Callable[[], Awaitable[Sequence[str]]]
) -> ReadScope | None:
    if auth.user_role in (Role.PROXY_ADMIN, Role.PROXY_ADMIN_VIEW_ONLY):
        return AllRows()
    if not auth.user_id:
        return None
    return OwnedRows(auth.user_id, tuple(await permitted_team_lookup()))

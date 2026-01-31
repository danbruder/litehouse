port module Ports exposing
    ( saveToken
    , saveRefreshToken
    , clearToken
    , getRefreshToken
    , refreshTokenReceived
    , connectSSE
    , disconnectSSE
    , updateSSEFilters
    , sseEvent
    , sseConnectionState
    )

import Json.Decode as Decode
import Json.Encode as Encode


-- Token management ports


port saveToken : String -> Cmd msg


port saveRefreshToken : String -> Cmd msg


port clearToken : () -> Cmd msg


port getRefreshToken : () -> Cmd msg


port refreshTokenReceived : (Maybe String -> msg) -> Sub msg


-- SSE (Server-Sent Events) ports


port connectSSE : { token : String, filters : Maybe Encode.Value } -> Cmd msg


port disconnectSSE : () -> Cmd msg


port updateSSEFilters : { filters : Maybe Encode.Value } -> Cmd msg


port sseEvent : (Decode.Value -> msg) -> Sub msg


port sseConnectionState : (String -> msg) -> Sub msg

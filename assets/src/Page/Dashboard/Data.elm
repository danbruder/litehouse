module Page.Dashboard.Data exposing
    ( UnifiedSSEMessage(..)
    , SSEEvent
    , unifiedSSEDecoder
    , sseEventDecoder
    , encodeAppFilter
    )

{-| Data module for Dashboard page.

This module contains all data types and decoders for API responses and SSE messages.

-}

import Json.Decode as Decode
import Json.Encode as Encode


-- TYPES


{-| Represents an SSE (Server-Sent Events) event with type and data.
-}
type alias SSEEvent =
    { eventType : String
    , data : String
    }


{-| Unified SSE message types representing all possible server events.
-}
type UnifiedSSEMessage
    = GitHubOAuthMessage String String -- eventType, data
    | BuildLogsMessage String String String String -- appName, buildId, eventType, data
    | BuildStatusMessage String String String -- appName, buildId, status
    | ContainerLogsMessage String String -- appName, data
    | AppStateMessage String String -- appName, state
    | SystemNotificationMessage String String -- level, message
    | HeartbeatMessage


-- DECODERS


{-| Decoder for basic SSE events.
-}
sseEventDecoder : Decode.Decoder SSEEvent
sseEventDecoder =
    Decode.map2 SSEEvent
        (Decode.field "type" Decode.string)
        (Decode.field "data" Decode.string)


{-| Unified decoder for all SSE message types.

This decoder parses the SSE message and routes it to the appropriate
UnifiedSSEMessage variant based on the message type.

-}
unifiedSSEDecoder : Decode.Decoder UnifiedSSEMessage
unifiedSSEDecoder =
    Decode.field "type" Decode.string
        |> Decode.andThen
            (\msgType ->
                case msgType of
                    "github_oauth" ->
                        Decode.map2 GitHubOAuthMessage
                            (Decode.at [ "data", "payload", "event_type" ] Decode.string)
                            (Decode.at [ "data", "payload", "data" ] Decode.string)

                    "build_logs" ->
                        Decode.map4 BuildLogsMessage
                            (Decode.at [ "data", "payload", "app_name" ] Decode.string)
                            (Decode.at [ "data", "payload", "build_id" ] Decode.string)
                            (Decode.at [ "data", "payload", "event_type" ] Decode.string)
                            (Decode.at [ "data", "payload", "data" ] Decode.string)

                    "build_status" ->
                        Decode.map3 BuildStatusMessage
                            (Decode.at [ "data", "payload", "app_name" ] Decode.string)
                            (Decode.at [ "data", "payload", "build_id" ] Decode.string)
                            (Decode.at [ "data", "payload", "status" ] Decode.string)

                    "container_logs" ->
                        Decode.map2 ContainerLogsMessage
                            (Decode.at [ "data", "payload", "app_name" ] Decode.string)
                            (Decode.at [ "data", "payload", "data" ] Decode.string)

                    "app_state" ->
                        Decode.map2 AppStateMessage
                            (Decode.at [ "data", "payload", "app_name" ] Decode.string)
                            (Decode.at [ "data", "payload", "state" ] Decode.string)

                    "system_notification" ->
                        Decode.map2 SystemNotificationMessage
                            (Decode.at [ "data", "payload", "level" ] Decode.string)
                            (Decode.at [ "data", "payload", "message" ] Decode.string)

                    "heartbeat" ->
                        Decode.succeed HeartbeatMessage

                    _ ->
                        Decode.fail ("Unknown SSE message type: " ++ msgType)
            )


-- ENCODERS


{-| Encode SSE filter for a specific app name.

This creates a filter that can be sent to the server to only receive
SSE events for the specified app.

-}
encodeAppFilter : String -> Encode.Value
encodeAppFilter appName =
    Encode.object
        [ ( "app_names", Encode.list Encode.string [ appName ] )
        ]

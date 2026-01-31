module Effects exposing (performEffect)

{-| This module contains the logic for executing Effects as Cmds.
It transforms our custom Effect type into actual side effects (Cmds).
-}

import Browser.Navigation as Nav
import Effect
import Http
import Json.Decode as Decode
import Json.Encode as Encode
import Ports


-- PERFORM EFFECT


performEffect : Nav.Key -> Effect.Effect msg -> Cmd msg
performEffect navKey effect =
    case effect of
        Effect.None ->
            Cmd.none

        Effect.Batch effects ->
            Cmd.batch (List.map (performEffect navKey) effects)

        Effect.PushUrl url ->
            Nav.pushUrl navKey url

        Effect.ReplaceUrl url ->
            Nav.replaceUrl navKey url

        Effect.Load href ->
            Nav.load href

        Effect.SaveToken token ->
            Ports.saveToken token

        Effect.SaveRefreshToken token ->
            Ports.saveRefreshToken token

        Effect.ClearToken ->
            Cmd.batch [ Ports.clearToken (), Ports.disconnectSSE () ]

        Effect.GetRefreshToken ->
            Ports.getRefreshToken ()

        Effect.ConnectSSE config ->
            Ports.connectSSE config

        Effect.DisconnectSSE ->
            Ports.disconnectSSE ()

        Effect.UpdateSSEFilters filters ->
            Ports.updateSSEFilters { filters = filters }

        Effect.CheckServerStatus toMsg ->
            Http.get
                { url = "/api/auth/status"
                , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) serverStatusDecoder
                }

        Effect.VerifyToken token toMsg ->
            Http.request
                { method = "GET"
                , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
                , url = "/api/auth/me"
                , body = Http.emptyBody
                , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) (tokenVerificationDecoder token)
                , timeout = Nothing
                , tracker = Nothing
                }

        Effect.RefreshAccessToken refreshToken toMsg ->
            Http.post
                { url = "/api/auth/refresh"
                , body =
                    Http.jsonBody
                        (Encode.object [ ( "refresh_token", Encode.string refreshToken ) ])
                , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) tokenPairDecoder
                }

        Effect.SubmitLogin form toMsg ->
            submitLoginHttp form toMsg

        Effect.SubmitRegister form toMsg ->
            submitRegisterHttp form toMsg

        Effect.FetchApps token toMsg ->
            fetchAppsHttp token toMsg

        Effect.FetchAppDetail token appName toMsg ->
            fetchAppDetailHttp token appName toMsg

        Effect.StartApp token appName toMsg ->
            startAppHttp token appName toMsg

        Effect.StopApp token appName toMsg ->
            stopAppHttp token appName toMsg

        Effect.BuildApp token appName toMsg ->
            buildAppHttp token appName toMsg

        Effect.DeleteApp token appName toMsg ->
            deleteAppHttp token appName toMsg

        Effect.FetchLogs token appName toMsg ->
            fetchLogsHttp token appName toMsg

        Effect.StartLogStreaming token appName toMsg ->
            startLogStreamingHttp token appName toMsg

        Effect.FetchBuilds token appName toMsg ->
            fetchBuildsHttp token appName toMsg

        Effect.FetchBuildLogs token appName buildId toMsg ->
            fetchBuildLogsHttp token appName buildId toMsg

        Effect.FetchEnvVars token appName toMsg ->
            fetchEnvVarsHttp token appName toMsg

        Effect.SetEnvVar token appName key value delete toMsg ->
            if delete then
                deleteEnvVarHttp token appName key toMsg
            else
                setEnvVarHttp token appName key value toMsg

        Effect.FetchGitHubStatus token toMsg ->
            fetchGitHubStatusHttp token toMsg

        Effect.StartDeviceFlow token toMsg ->
            startDeviceFlowHttp token toMsg

        Effect.StartGitHubPolling token deviceCode interval expiresIn ->
            startGitHubPollingHttp token deviceCode interval expiresIn

        Effect.FetchRepos token toMsg ->
            fetchReposHttp token toMsg

        Effect.CreateApp token name toMsg ->
            createAppHttp token name toMsg

        Effect.CreateAppWithRepo token name repo toMsg ->
            createAppWithRepoHttp token name repo toMsg

        Effect.FetchS3Config token toMsg ->
            fetchS3ConfigHttp token toMsg

        Effect.SetS3Config token form toMsg ->
            setS3ConfigHttp token form toMsg

        Effect.DeleteS3Config token toMsg ->
            deleteS3ConfigHttp token toMsg

        Effect.UpdateGitHubStatus _ ->
            -- Handled by updateSharedFromEffect, no Cmd needed
            Cmd.none



-- HTTP REQUEST FUNCTIONS


submitLoginHttp : Effect.LoginForm -> (Result String Effect.AuthResponse -> msg) -> Cmd msg
submitLoginHttp form toMsg =
    Http.post
        { url = "/api/auth/login"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "email", Encode.string form.email )
                    , ( "password", Encode.string form.password )
                    ]
                )
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) authResponseDecoder
        }


submitRegisterHttp : Effect.SetupForm -> (Result String Effect.AuthResponse -> msg) -> Cmd msg
submitRegisterHttp form toMsg =
    Http.post
        { url = "/api/auth/register"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "email", Encode.string form.email )
                    , ( "password", Encode.string form.password )
                    , ( "full_name", Encode.string form.fullName )
                    , ( "organization_name", Encode.string form.organizationName )
                    ]
                )
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) authResponseDecoder
        }


fetchAppsHttp : String -> (Result String (List Effect.AppInfo) -> msg) -> Cmd msg
fetchAppsHttp token toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps"
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) appsListDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


fetchGitHubStatusHttp : String -> (Result String Effect.GitHubStatusResponse -> msg) -> Cmd msg
fetchGitHubStatusHttp token toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/github/status"
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) githubStatusDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


startDeviceFlowHttp : String -> (Result String Effect.DeviceFlowStartResponse -> msg) -> Cmd msg
startDeviceFlowHttp token toMsg =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/github/connect/start"
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) deviceFlowStartDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


startGitHubPollingHttp : String -> String -> Int -> Int -> Cmd msg
startGitHubPollingHttp token deviceCode interval expiresIn =
    -- Fire-and-forget polling request - response comes via SSE
    -- We ignore the HTTP response since events will be streamed via SSE
    Cmd.none


fetchReposHttp : String -> (Result String (List Effect.RepoInfo) -> msg) -> Cmd msg
fetchReposHttp token toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/github/repos"
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) reposListDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


createAppHttp : String -> String -> (Result String Effect.AppInfo -> msg) -> Cmd msg
createAppHttp token name toMsg =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "name", Encode.string name )
                    ]
                )
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) appInfoDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


createAppWithRepoHttp : String -> String -> String -> (Result String Effect.AppInfo -> msg) -> Cmd msg
createAppWithRepoHttp token name repoFullName toMsg =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "name", Encode.string name )
                    , ( "from_github", Encode.string repoFullName )
                    ]
                )
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) appInfoDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


fetchAppDetailHttp : String -> String -> (Result String Effect.AppDetail -> msg) -> Cmd msg
fetchAppDetailHttp token appName toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) appDetailDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


startAppHttp : String -> String -> (Result String String -> msg) -> Cmd msg
startAppHttp token appName toMsg =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/start"
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }


stopAppHttp : String -> String -> (Result String String -> msg) -> Cmd msg
stopAppHttp token appName toMsg =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/stop"
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }


buildAppHttp : String -> String -> (Result String String -> msg) -> Cmd msg
buildAppHttp token appName toMsg =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/build"
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Just 300000
        , tracker = Nothing
        }


deleteAppHttp : String -> String -> (Result String String -> msg) -> Cmd msg
deleteAppHttp token appName toMsg =
    Http.request
        { method = "DELETE"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }


fetchLogsHttp : String -> String -> (Result String String -> msg) -> Cmd msg
fetchLogsHttp token appName toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/logs?lines=100"
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }


startLogStreamingHttp : String -> String -> (Result String String -> msg) -> Cmd msg
startLogStreamingHttp token appName toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/logs?lines=50&follow=true"
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }


fetchBuildsHttp : String -> String -> (Result String (List Effect.BuildInfo) -> msg) -> Cmd msg
fetchBuildsHttp token appName toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/builds"
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) (Decode.list buildInfoDecoder)
        , timeout = Nothing
        , tracker = Nothing
        }


fetchBuildLogsHttp : String -> String -> String -> (Result String String -> msg) -> Cmd msg
fetchBuildLogsHttp token appName buildId toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/builds/" ++ buildId ++ "/logs"
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }


fetchEnvVarsHttp : String -> String -> (Result String (List Effect.EnvVar) -> msg) -> Cmd msg
fetchEnvVarsHttp token appName toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/env"
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) (Decode.list envVarDecoder)
        , timeout = Nothing
        , tracker = Nothing
        }


setEnvVarHttp : String -> String -> String -> String -> (Result String String -> msg) -> Cmd msg
setEnvVarHttp token appName key value toMsg =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/env"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "key", Encode.string key )
                    , ( "value", Encode.string value )
                    , ( "delete", Encode.bool False )
                    ]
                )
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }


deleteEnvVarHttp : String -> String -> String -> (Result String String -> msg) -> Cmd msg
deleteEnvVarHttp token appName key toMsg =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/env"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "key", Encode.string key )
                    , ( "value", Encode.string "" )
                    , ( "delete", Encode.bool True )
                    ]
                )
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }


fetchS3ConfigHttp : String -> (Result String (Maybe Effect.S3ConfigRedacted) -> msg) -> Cmd msg
fetchS3ConfigHttp token toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/config/s3"
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) (Decode.nullable s3ConfigRedactedDecoder)
        , timeout = Nothing
        , tracker = Nothing
        }


setS3ConfigHttp : String -> Effect.S3ConfigForm -> (Result String String -> msg) -> Cmd msg
setS3ConfigHttp token form toMsg =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/config/s3"
        , body =
            Http.jsonBody
                (Encode.object
                    (List.filterMap identity
                        [ Just ( "access_key_id", Encode.string form.accessKeyId )
                        , Just ( "secret_access_key", Encode.string form.secretAccessKey )
                        , Just ( "bucket", Encode.string form.bucket )
                        , Just ( "region", Encode.string form.region )
                        , if String.isEmpty form.endpoint then
                            Nothing
                          else
                            Just ( "endpoint", Encode.string form.endpoint )
                        , if String.isEmpty form.pathPrefix then
                            Nothing
                          else
                            Just ( "path_prefix", Encode.string form.pathPrefix )
                        ]
                    )
                )
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }


deleteS3ConfigHttp : String -> (Result String String -> msg) -> Cmd msg
deleteS3ConfigHttp token toMsg =
    Http.request
        { method = "DELETE"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/config/s3"
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }



-- DECODERS


type alias ServerStatus =
    { initialized : Bool
    , version : String
    }


serverStatusDecoder : Decode.Decoder ServerStatus
serverStatusDecoder =
    Decode.map2 ServerStatus
        (Decode.field "initialized" Decode.bool)
        (Decode.field "version" Decode.string)


buildInfoDecoder : Decode.Decoder Effect.BuildInfo
buildInfoDecoder =
    Decode.map6 Effect.BuildInfo
        (Decode.field "id" Decode.string)
        (Decode.field "app_id" Decode.string)
        (Decode.field "image_tag" (Decode.nullable Decode.string))
        (Decode.field "git_commit" (Decode.nullable Decode.string))
        (Decode.field "status" Decode.string)
        (Decode.field "created_at" Decode.string)


type alias TokenVerificationResponse =
    { user : Effect.UserInfo
    , token : String
    }


tokenVerificationDecoder : String -> Decode.Decoder TokenVerificationResponse
tokenVerificationDecoder token =
    Decode.map2 TokenVerificationResponse
        (Decode.map2 Effect.UserInfo
            (Decode.at [ "user", "email" ] Decode.string)
            (Decode.at [ "user", "full_name" ] Decode.string)
        )
        (Decode.succeed token)


authResponseDecoder : Decode.Decoder Effect.AuthResponse
authResponseDecoder =
    Decode.map3 Effect.AuthResponse
        (Decode.at [ "tokens", "access_token" ] Decode.string)
        (Decode.at [ "tokens", "refresh_token" ] Decode.string)
        (Decode.field "user" userDecoder)


type alias TokenPair =
    { accessToken : String
    , refreshToken : String
    }


tokenPairDecoder : Decode.Decoder TokenPair
tokenPairDecoder =
    Decode.map2 TokenPair
        (Decode.field "access_token" Decode.string)
        (Decode.field "refresh_token" Decode.string)


userDecoder : Decode.Decoder Effect.UserInfo
userDecoder =
    Decode.map2 Effect.UserInfo
        (Decode.field "email" Decode.string)
        (Decode.field "full_name" Decode.string)


appsListDecoder : Decode.Decoder (List Effect.AppInfo)
appsListDecoder =
    Decode.list appInfoDecoder


appInfoDecoder : Decode.Decoder Effect.AppInfo
appInfoDecoder =
    Decode.map3 Effect.AppInfo
        (Decode.field "id" Decode.string)
        (Decode.field "name" Decode.string)
        (Decode.field "state" Decode.string)


githubStatusDecoder : Decode.Decoder Effect.GitHubStatusResponse
githubStatusDecoder =
    Decode.map2 Effect.GitHubStatusResponse
        (Decode.field "connected" Decode.bool)
        (Decode.maybe (Decode.field "username" Decode.string))


deviceFlowStartDecoder : Decode.Decoder Effect.DeviceFlowStartResponse
deviceFlowStartDecoder =
    Decode.map5 Effect.DeviceFlowStartResponse
        (Decode.field "user_code" Decode.string)
        (Decode.field "verification_uri" Decode.string)
        (Decode.field "device_code" Decode.string)
        (Decode.field "expires_in" Decode.int)
        (Decode.field "interval" Decode.int)


reposListDecoder : Decode.Decoder (List Effect.RepoInfo)
reposListDecoder =
    Decode.list repoInfoDecoder


repoInfoDecoder : Decode.Decoder Effect.RepoInfo
repoInfoDecoder =
    Decode.map6 Effect.RepoInfo
        (Decode.field "name" Decode.string)
        (Decode.field "full_name" Decode.string)
        (Decode.maybe (Decode.field "description" Decode.string))
        (Decode.field "private" Decode.bool)
        (Decode.field "clone_url" Decode.string)
        (Decode.field "default_branch" Decode.string)


appDetailDecoder : Decode.Decoder Effect.AppDetail
appDetailDecoder =
    Decode.map7 Effect.AppDetail
        (Decode.field "id" Decode.string)
        (Decode.field "name" Decode.string)
        (Decode.field "state" Decode.string)
        (Decode.maybe (Decode.field "port" Decode.int))
        (Decode.field "created_at" Decode.string)
        (Decode.field "updated_at" Decode.string)
        (Decode.maybe (Decode.field "remote" remoteInfoDecoder))


envVarDecoder : Decode.Decoder Effect.EnvVar
envVarDecoder =
    Decode.map2 Effect.EnvVar
        (Decode.field "key" Decode.string)
        (Decode.field "value" Decode.string)


remoteInfoDecoder : Decode.Decoder Effect.RemoteInfo
remoteInfoDecoder =
    Decode.map3 Effect.RemoteInfo
        (Decode.field "name" Decode.string)
        (Decode.field "url" Decode.string)
        (Decode.field "branch" Decode.string)


s3ConfigRedactedDecoder : Decode.Decoder Effect.S3ConfigRedacted
s3ConfigRedactedDecoder =
    Decode.map5 Effect.S3ConfigRedacted
        (Decode.field "access_key_id" Decode.string)
        (Decode.field "bucket" Decode.string)
        (Decode.field "region" Decode.string)
        (Decode.maybe (Decode.field "endpoint" Decode.string))
        (Decode.maybe (Decode.field "path_prefix" Decode.string))



-- HELPERS


httpErrorToString : Http.Error -> String
httpErrorToString error =
    case error of
        Http.BadUrl url ->
            "Invalid URL: " ++ url

        Http.Timeout ->
            "Request timed out"

        Http.NetworkError ->
            "Network error - please check your connection"

        Http.BadStatus status ->
            case status of
                401 ->
                    "Invalid email or password"

                409 ->
                    "An account with this email already exists"

                _ ->
                    "Server error (status " ++ String.fromInt status ++ ")"

        Http.BadBody message ->
            "Error parsing response: " ++ message

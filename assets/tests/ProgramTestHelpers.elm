module ProgramTestHelpers exposing
    ( createTestProgram
    , createSimulateEffects
    , ensureHttpRequest
    , ensureHttpRequestWithBody
    , simulateHttpOk
    , simulateHttpError
    , ensureViewHasText
    , ensureViewHasNotText
    , clickButton
    , clickLink
    , fillIn
    , submitForm
    , authResponseJson
    , serverStatusJson
    , appsListJson
    , appDetailJson
    , githubStatusJson
    , reposListJson
    , ensureBrowserUrlPath
    )

{-| Helper functions for writing integration tests with elm-program-test.
These follow best practices for testing Elm applications.
-}

import Browser
import Browser.Navigation as Nav
import Dict
import Effect
import Expect
import Http
import Json.Decode as Decode
import Json.Encode as Encode
import Main
import ProgramTest exposing (ProgramTest, SimulatedEffect)
import SimulatedEffect.Cmd
import SimulatedEffect.Http
import SimulatedEffect.Navigation
import SimulatedEffect.Ports
import Test.Html.Query as Query
import Test.Html.Selector exposing (attribute, class, containing, tag, text)
import Url


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


-- Decoders (recreated from Main.elm for use in tests)
appInfoDecoder : Decode.Decoder Effect.AppInfo
appInfoDecoder =
    Decode.map3 Effect.AppInfo
        (Decode.field "id" Decode.string)
        (Decode.field "name" Decode.string)
        (Decode.field "state" Decode.string)


appsListDecoder : Decode.Decoder (List Effect.AppInfo)
appsListDecoder =
    Decode.list appInfoDecoder


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


remoteInfoDecoder : Decode.Decoder Effect.RemoteInfo
remoteInfoDecoder =
    Decode.map3 Effect.RemoteInfo
        (Decode.field "name" Decode.string)
        (Decode.field "url" Decode.string)
        (Decode.field "branch" Decode.string)


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


buildInfoDecoder : Decode.Decoder Effect.BuildInfo
buildInfoDecoder =
    Decode.map6 Effect.BuildInfo
        (Decode.field "id" Decode.string)
        (Decode.field "app_id" Decode.string)
        (Decode.field "image_tag" (Decode.nullable Decode.string))
        (Decode.field "git_commit" (Decode.nullable Decode.string))
        (Decode.field "status" Decode.string)
        (Decode.field "created_at" Decode.string)


envVarDecoder : Decode.Decoder Effect.EnvVar
envVarDecoder =
    Decode.map2 Effect.EnvVar
        (Decode.field "key" Decode.string)
        (Decode.field "value" Decode.string)


s3ConfigRedactedDecoder : Decode.Decoder Effect.S3ConfigRedacted
s3ConfigRedactedDecoder =
    Decode.map5 Effect.S3ConfigRedacted
        (Decode.field "access_key_id" Decode.string)
        (Decode.field "bucket" Decode.string)
        (Decode.field "region" Decode.string)
        (Decode.maybe (Decode.field "endpoint" Decode.string))
        (Decode.maybe (Decode.field "path_prefix" Decode.string))


authResponseDecoder : Decode.Decoder Effect.AuthResponse
authResponseDecoder =
    Decode.map3 Effect.AuthResponse
        (Decode.at [ "tokens", "access_token" ] Decode.string)
        (Decode.at [ "tokens", "refresh_token" ] Decode.string)
        (Decode.field "user" userDecoder)


userDecoder : Decode.Decoder Effect.UserInfo
userDecoder =
    Decode.map2 Effect.UserInfo
        (Decode.field "email" Decode.string)
        (Decode.field "full_name" Decode.string)


tokenPairDecoder : Decode.Decoder { accessToken : String, refreshToken : String }
tokenPairDecoder =
    Decode.map2 (\access refresh -> { accessToken = access, refreshToken = refresh })
        (Decode.field "access_token" Decode.string)
        (Decode.field "refresh_token" Decode.string)


serverStatusDecoder : Decode.Decoder { initialized : Bool, version : String }
serverStatusDecoder =
    Decode.map2 (\init ver -> { initialized = init, version = ver })
        (Decode.field "initialized" Decode.bool)
        (Decode.field "version" Decode.string)


tokenVerificationDecoder : String -> Decode.Decoder { user : Effect.UserInfo, token : String }
tokenVerificationDecoder token =
    Decode.map (\user -> { user = user, token = token })
        (Decode.map2 Effect.UserInfo
            (Decode.at [ "user", "email" ] Decode.string)
            (Decode.at [ "user", "full_name" ] Decode.string)
        )


{-| Create a function to simulate effects for testing.
-}
createSimulateEffects : () -> Effect.Effect Main.Msg -> SimulatedEffect Main.Msg
createSimulateEffects () effect =
    case effect of
                Effect.None ->
                    SimulatedEffect.Cmd.none

                Effect.Batch effects ->
                    SimulatedEffect.Cmd.batch (List.map (createSimulateEffects ()) effects)

                Effect.PushUrl url ->
                    SimulatedEffect.Navigation.pushUrl url

                Effect.ReplaceUrl url ->
                    SimulatedEffect.Navigation.replaceUrl url

                Effect.SaveToken token ->
                    SimulatedEffect.Ports.send "saveToken" (Encode.string token)

                Effect.SaveRefreshToken token ->
                    SimulatedEffect.Ports.send "saveRefreshToken" (Encode.string token)

                Effect.ClearToken ->
                    SimulatedEffect.Cmd.batch
                        [ SimulatedEffect.Ports.send "clearToken" (Encode.null)
                        , SimulatedEffect.Ports.send "disconnectSSE" (Encode.null)
                        ]

                Effect.GetRefreshToken ->
                    SimulatedEffect.Ports.send "getRefreshToken" (Encode.null)

                Effect.ConnectSSE config ->
                    SimulatedEffect.Ports.send "connectSSE"
                        (Encode.object
                            [ ( "token", Encode.string config.token )
                            , ( "filters"
                              , case config.filters of
                                    Just filters ->
                                        filters

                                    Nothing ->
                                        Encode.null
                              )
                            ]
                        )

                Effect.DisconnectSSE ->
                    SimulatedEffect.Ports.send "disconnectSSE" (Encode.null)

                Effect.UpdateSSEFilters filters ->
                    SimulatedEffect.Ports.send "updateSSEFilters"
                        (Encode.object
                            [ ( "filters"
                              , case filters of
                                    Just filterValue ->
                                        filterValue

                                    Nothing ->
                                        Encode.null
                              )
                            ]
                        )

                Effect.CheckServerStatus toMsg ->
                    SimulatedEffect.Http.get
                        { url = "/api/auth/status"
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) serverStatusDecoder
                        }

                Effect.VerifyToken token toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/auth/me"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) (tokenVerificationDecoder token)
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.RefreshAccessToken refreshToken toMsg ->
                    SimulatedEffect.Http.post
                        { url = "/api/auth/refresh"
                        , body = SimulatedEffect.Http.jsonBody
                            (Encode.object [ ( "refresh_token", Encode.string refreshToken ) ])
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) tokenPairDecoder
                        }

                Effect.SubmitLogin form toMsg ->
                    SimulatedEffect.Http.post
                        { url = "/api/auth/login"
                        , body = SimulatedEffect.Http.jsonBody
                            (Encode.object
                                [ ( "email", Encode.string form.email )
                                , ( "password", Encode.string form.password )
                                ]
                            )
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) authResponseDecoder
                        }

                Effect.SubmitRegister form toMsg ->
                    SimulatedEffect.Http.post
                        { url = "/api/auth/register"
                        , body = SimulatedEffect.Http.jsonBody
                            (Encode.object
                                [ ( "email", Encode.string form.email )
                                , ( "password", Encode.string form.password )
                                , ( "full_name", Encode.string form.fullName )
                                , ( "organization_name", Encode.string form.organizationName )
                                ]
                            )
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) authResponseDecoder
                        }

                Effect.FetchApps token toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) appsListDecoder
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.FetchAppDetail token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) appDetailDecoder
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.StartApp token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "POST"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/start"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (\result -> toMsg (Result.mapError httpErrorToString result))
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.StopApp token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "POST"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/stop"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (\result -> toMsg (Result.mapError httpErrorToString result))
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.BuildApp token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "POST"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/build"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (\result -> toMsg (Result.mapError httpErrorToString result))
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.DeleteApp token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "DELETE"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (\result -> toMsg (Result.mapError httpErrorToString result))
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.FetchLogs token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/logs?lines=100"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (\result -> toMsg (Result.mapError httpErrorToString result))
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.StartLogStreaming token appName toMsg ->
                    -- StartLogStreaming doesn't return immediately, it's handled via SSE
                    -- So we'll just return none for testing
                    SimulatedEffect.Cmd.none

                Effect.FetchBuilds token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/builds"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) (Decode.list buildInfoDecoder)
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.FetchBuildLogs token appName buildId toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/builds/" ++ buildId ++ "/logs"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (\result -> toMsg (Result.mapError httpErrorToString result))
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.FetchGitHubStatus token toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/github/status"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) githubStatusDecoder
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.StartDeviceFlow token toMsg ->
                    SimulatedEffect.Http.request
                        { method = "POST"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/github/connect/start"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) deviceFlowStartDecoder
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.StartGitHubPolling token deviceCode interval expiresIn ->
                    -- StartGitHubPolling doesn't have a toMsg parameter, so we can't simulate it
                    -- This effect is handled via SSE, so we'll just return none
                    SimulatedEffect.Cmd.none

                Effect.FetchRepos token toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/github/repos"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) reposListDecoder
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.CreateApp token name toMsg ->
                    SimulatedEffect.Http.request
                        { method = "POST"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps"
                        , body = SimulatedEffect.Http.jsonBody
                            (Encode.object [ ( "name", Encode.string name ) ])
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) appInfoDecoder
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.CreateAppWithRepo token name repo toMsg ->
                    SimulatedEffect.Http.request
                        { method = "POST"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps"
                        , body = SimulatedEffect.Http.jsonBody
                            (Encode.object
                                [ ( "name", Encode.string name )
                                , ( "from_github", Encode.string repo )
                                ]
                            )
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) appInfoDecoder
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.Load href ->
                    SimulatedEffect.Navigation.load href

                Effect.FetchEnvVars token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/env"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) (Decode.list envVarDecoder)
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.SetEnvVar token appName key value delete toMsg ->
                    SimulatedEffect.Http.request
                        { method = "POST"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/env"
                        , body = SimulatedEffect.Http.jsonBody
                            (Encode.object
                                [ ( "key", Encode.string key )
                                , ( "value", Encode.string value )
                                , ( "delete", Encode.bool delete )
                                ]
                            )
                        , expect = SimulatedEffect.Http.expectString (\result -> toMsg (Result.mapError httpErrorToString result))
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.FetchS3Config token toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/config/s3"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectJson (\result -> toMsg (Result.mapError httpErrorToString result)) (Decode.nullable s3ConfigRedactedDecoder)
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.SetS3Config token form toMsg ->
                    SimulatedEffect.Http.request
                        { method = "POST"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/config/s3"
                        , body = SimulatedEffect.Http.jsonBody
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
                        , expect = SimulatedEffect.Http.expectString (\result -> toMsg (Result.mapError httpErrorToString result))
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.DeleteS3Config token toMsg ->
                    SimulatedEffect.Http.request
                        { method = "DELETE"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/config/s3"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (\result -> toMsg (Result.mapError httpErrorToString result))
                        , timeout = Nothing
                        , tracker = Nothing
                        }

                Effect.UpdateGitHubStatus _ ->
                    SimulatedEffect.Cmd.none


createTestProgram : ProgramTest (Main.Model ()) Main.Msg (Effect.Effect Main.Msg)
createTestProgram =
    let
        flags =
            { token = Nothing }
    in
    ProgramTest.createApplication
        { init = Main.initForTesting
        , update = Main.update
        , view = Main.view
        , onUrlChange = Main.UrlChanged
        , onUrlRequest = Main.LinkClicked
        }
        |> ProgramTest.withBaseUrl "http://localhost"
        |> ProgramTest.withSimulatedEffects (createSimulateEffects ())
        |> ProgramTest.start flags


{-| Ensure that an HTTP request was made with the given method and URL.
Returns ProgramTest so it can be chained.
-}
ensureHttpRequest : String -> String -> ProgramTest model msg effect -> ProgramTest model msg effect
ensureHttpRequest method url programTest =
    ProgramTest.ensureHttpRequestWasMade method url programTest


{-| Ensure that an HTTP request was made with the given method, URL, and JSON body.
-}
ensureHttpRequestWithBody : String -> String -> Encode.Value -> ProgramTest model msg effect -> ProgramTest model msg effect
ensureHttpRequestWithBody method url expectedBody programTest =
    ProgramTest.ensureHttpRequest method
        url
        (\request ->
            request.body
                |> Expect.equal (Encode.encode 0 expectedBody)
        )
        programTest


{-| Simulate an HTTP response with status 200 and JSON body.
-}
simulateHttpOk : String -> String -> Encode.Value -> ProgramTest model msg effect -> ProgramTest model msg effect
simulateHttpOk method url responseBody programTest =
    ProgramTest.simulateHttpOk method
        url
        (Encode.encode 0 responseBody)
        programTest


{-| Simulate an HTTP error response.
-}
simulateHttpError : String -> String -> Http.Error -> ProgramTest model msg effect -> ProgramTest model msg effect
simulateHttpError method url error programTest =
    let
        metadata statusCode =
            { url = url
            , statusCode = statusCode
            , statusText = ""
            , headers = Dict.empty
            }
    in
    case error of
        Http.BadStatus statusCode ->
            ProgramTest.simulateHttpResponse method
                url
                (Http.BadStatus_ (metadata statusCode) "")
                programTest

        Http.NetworkError ->
            ProgramTest.simulateHttpResponse method
                url
                Http.NetworkError_
                programTest

        Http.Timeout ->
            ProgramTest.simulateHttpResponse method
                url
                Http.Timeout_
                programTest

        Http.BadUrl badUrl ->
            ProgramTest.simulateHttpResponse method
                url
                (Http.BadUrl_ badUrl)
                programTest

        Http.BadBody body ->
            ProgramTest.simulateHttpResponse method
                url
                (Http.BadStatus_ (metadata 400) body)
                programTest


{-| Ensure that the view contains the given text.
Returns ProgramTest so it can be chained.
-}
ensureViewHasText : String -> ProgramTest model msg effect -> ProgramTest model msg effect
ensureViewHasText expectedText programTest =
    ProgramTest.ensureView
        (Query.has [ text expectedText ])
        programTest


{-| Ensure that the view does not contain the given text.
Returns ProgramTest so it can be chained.
-}
ensureViewHasNotText : String -> ProgramTest model msg effect -> ProgramTest model msg effect
ensureViewHasNotText unexpectedText programTest =
    ProgramTest.ensureView
        (Query.hasNot [ text unexpectedText ])
        programTest


{-| Click a button with the given text.
If multiple buttons with the same text exist, clicks the first one.
-}
clickButton : String -> ProgramTest model msg effect -> ProgramTest model msg effect
clickButton buttonText programTest =
    -- Use simulateDomEvent to have more control over which button to click
    ProgramTest.simulateDomEvent
        (\query ->
            Query.findAll [ tag "button", containing [ text buttonText ] ] query
                |> Query.index 0
        )
        ( "click", Encode.object [] )
        programTest


{-| Click a link (anchor element) that contains the given text.
Finds the first anchor element containing the text and clicks it.
For app cards, looks for links with the app card styling to avoid clicking header links.
-}
clickLink : String -> ProgramTest model msg effect -> ProgramTest model msg effect
clickLink linkText programTest =
    -- Find app card links (they have specific classes like "block" and "bg-litehouse-surface")
    -- Use findAll to get all matching links, then take the first one
    ProgramTest.simulateDomEvent
        (\query ->
            Query.findAll [ tag "a", containing [ text linkText ] ] query
                |> Query.index 0
        )
        ( "click", Encode.object [] )
        programTest


{-| Fill in an input field with the given field ID.
The fieldId should match the HTML id attribute of the input.
We map common fieldIds to their label text for ProgramTest.fillIn.
-}
fillIn : String -> String -> ProgramTest model msg effect -> ProgramTest model msg effect
fillIn fieldId value programTest =
    -- fillIn takes: fieldId, label, newContent, programTest
    -- Map fieldIds to their actual HTML id and label text
    let
        ( actualFieldId, labelText ) =
            case fieldId of
                "appName" ->
                    ( "appName", "App Name" )

                "email" ->
                    ( "email", "Email" )

                "password" ->
                    ( "password", "Password" )

                "Password" ->
                    ( "password", "Password" )

                "fullName" ->
                    ( "fullName", "Full Name" )

                "orgName" ->
                    ( "orgName", "Organization Name" )

                "confirmPassword" ->
                    ( "confirmPassword", "Confirm Password" )

                "Confirm Password" ->
                    ( "confirmPassword", "Confirm Password" )

                _ ->
                    ( fieldId, fieldId )
    in
    ProgramTest.fillIn actualFieldId labelText value programTest


{-| Submit a form by finding the form element and triggering submit.
-}
submitForm : ProgramTest model msg effect -> ProgramTest model msg effect
submitForm programTest =
    ProgramTest.simulateDomEvent
        (Query.find [ tag "form" ])
        ( "submit", Encode.object [] )
        programTest


{-| Helper to create a JSON response for auth responses.
-}
authResponseJson : String -> String -> Effect.UserInfo -> Encode.Value
authResponseJson accessToken refreshToken user =
    Encode.object
        [ ( "tokens"
          , Encode.object
                [ ( "access_token", Encode.string accessToken )
                , ( "refresh_token", Encode.string refreshToken )
                ]
          )
        , ( "user"
          , Encode.object
                [ ( "email", Encode.string user.email )
                , ( "full_name", Encode.string user.fullName )
                ]
          )
        ]


{-| Helper to create a JSON response for server status.
-}
serverStatusJson : Bool -> String -> Encode.Value
serverStatusJson initialized version =
    Encode.object
        [ ( "initialized", Encode.bool initialized )
        , ( "version", Encode.string version )
        ]


{-| Helper to create a JSON response for apps list.
-}
appsListJson : List Effect.AppInfo -> Encode.Value
appsListJson apps =
    Encode.list
        (\app ->
            Encode.object
                [ ( "id", Encode.string app.id )
                , ( "name", Encode.string app.name )
                , ( "state", Encode.string app.state )
                ]
        )
        apps


{-| Helper to create a JSON response for app detail.
-}
appDetailJson : Effect.AppDetail -> Encode.Value
appDetailJson app =
    Encode.object
        [ ( "id", Encode.string app.id )
        , ( "name", Encode.string app.name )
        , ( "state", Encode.string app.state )
        , ( "port"
          , case app.port_ of
                Just port_ ->
                    Encode.int port_

                Nothing ->
                    Encode.null
          )
        , ( "created_at", Encode.string app.createdAt )
        , ( "updated_at", Encode.string app.updatedAt )
        , ( "remote"
          , case app.remote of
                Just remote ->
                    Encode.object
                        [ ( "name", Encode.string remote.name )
                        , ( "url", Encode.string remote.url )
                        , ( "branch", Encode.string remote.branch )
                        ]

                Nothing ->
                    Encode.null
          )
        ]


{-| Helper to create a JSON response for GitHub status.
-}
githubStatusJson : Bool -> Maybe String -> Encode.Value
githubStatusJson connected maybeUsername =
    Encode.object
        [ ( "connected", Encode.bool connected )
        , ( "username"
          , case maybeUsername of
                Just username ->
                    Encode.string username

                Nothing ->
                    Encode.null
          )
        ]


{-| Helper to create a JSON response for repos list.
-}
reposListJson : List Effect.RepoInfo -> Encode.Value
reposListJson repos =
    Encode.list
        (\repo ->
            Encode.object
                [ ( "name", Encode.string repo.name )
                , ( "full_name", Encode.string repo.fullName )
                , ( "description"
                  , case repo.description of
                        Just desc ->
                            Encode.string desc

                        Nothing ->
                            Encode.null
                  )
                , ( "private", Encode.bool repo.private )
                , ( "clone_url", Encode.string repo.cloneUrl )
                , ( "default_branch", Encode.string repo.defaultBranch )
                ]
        )
        repos


{-| Ensure that the browser URL path matches the expected path.
Extracts the path from the full URL for comparison.
-}
ensureBrowserUrlPath : String -> ProgramTest model msg effect -> ProgramTest model msg effect
ensureBrowserUrlPath expectedPath programTest =
    ProgramTest.ensureBrowserUrl
        (\urlString ->
            case Url.fromString urlString of
                Just url ->
                    Expect.equal expectedPath url.path

                Nothing ->
                    Expect.fail ("Could not parse URL: " ++ urlString)
        )
        programTest

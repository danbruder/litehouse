module ProgramTestHelpers exposing
    ( createTestProgram
    , ensureHttpRequest
    , ensureHttpRequestWithBody
    , simulateHttpOk
    , simulateHttpError
    , ensureViewHasText
    , ensureViewHasNotText
    , clickButton
    , fillIn
    , submitForm
    , authResponseJson
    , serverStatusJson
    , appsListJson
    , appDetailJson
    , githubStatusJson
    , reposListJson
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
import Json.Encode as Encode
import Main
import ProgramTest exposing (ProgramTest, SimulatedEffect)
import SimulatedEffect.Cmd
import SimulatedEffect.Http
import SimulatedEffect.Navigation
import SimulatedEffect.Ports
import Test.Html.Query as Query
import Test.Html.Selector exposing (text)
import Url


{-| Create a ProgramTest instance for the application with default flags.
Wraps Main.init to match the createApplication signature.
-}
createTestProgram : ProgramTest (Main.Model ()) Main.Msg (Effect.Effect Main.Msg)
createTestProgram =
    let
        flags =
            { token = Nothing }

        -- Convert Effect to SimulatedEffect for testing
        simulateEffects : Effect.Effect Main.Msg -> SimulatedEffect Main.Msg
        simulateEffects effect =
            case effect of
                Effect.None ->
                    SimulatedEffect.Cmd.none

                Effect.Batch effects ->
                    SimulatedEffect.Cmd.batch (List.map simulateEffects effects)

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

                Effect.CheckServerStatus toMsg ->
                    SimulatedEffect.Http.get
                        { url = "/api/auth/status"
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.VerifyToken token toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/auth/me"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.RefreshAccessToken refreshToken toMsg ->
                    SimulatedEffect.Http.post
                        { url = "/api/auth/refresh"
                        , body = SimulatedEffect.Http.jsonBody
                            (Encode.object [ ( "refresh_token", Encode.string refreshToken ) ])
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
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
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
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
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.FetchApps token toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.FetchAppDetail token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.StartApp token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "POST"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/start"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.StopApp token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "POST"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/stop"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.BuildApp token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "POST"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/build"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.DeleteApp token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "DELETE"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.FetchLogs token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/logs?lines=100"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.FetchBuilds token appName toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/builds"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.FetchBuildLogs token appName buildId toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps/" ++ appName ++ "/builds/" ++ buildId ++ "/logs"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.FetchGitHubStatus token toMsg ->
                    SimulatedEffect.Http.request
                        { method = "GET"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/github/status"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.StartDeviceFlow token toMsg ->
                    SimulatedEffect.Http.request
                        { method = "POST"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/github/connect/start"
                        , body = SimulatedEffect.Http.emptyBody
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
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
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.CreateApp token name toMsg ->
                    SimulatedEffect.Http.request
                        { method = "POST"
                        , headers = [ ( "Authorization", "Bearer " ++ token ) ]
                        , url = "/api/apps"
                        , body = SimulatedEffect.Http.jsonBody
                            (Encode.object [ ( "name", Encode.string name ) ])
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
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
                        , expect = SimulatedEffect.Http.expectString (toMsg << Ok)
                        }

                Effect.UpdateGitHubStatus _ ->
                    SimulatedEffect.Cmd.none
    in
    ProgramTest.createApplication
        { init = Main.initForTesting
        , update = Main.update
        , view = Main.view
        , onUrlChange = Main.UrlChanged
        , onUrlRequest = Main.LinkClicked
        }
        |> ProgramTest.withBaseUrl "http://localhost"
        |> ProgramTest.withSimulatedEffects simulateEffects
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
-}
clickButton : String -> ProgramTest model msg effect -> ProgramTest model msg effect
clickButton buttonText programTest =
    ProgramTest.clickButton buttonText programTest


{-| Fill in an input field with the given label.
-}
fillIn : String -> String -> ProgramTest model msg effect -> ProgramTest model msg effect
fillIn label value programTest =
    -- fillIn takes: fieldId, label, newContent, programTest
    -- We'll use label as both fieldId and label for simplicity
    ProgramTest.fillIn label label value programTest


{-| Submit a form by finding the form element and triggering submit.
-}
submitForm : ProgramTest model msg effect -> ProgramTest model msg effect
submitForm programTest =
    ProgramTest.within
        (Query.find [])
        (\test ->
            ProgramTest.simulateDomEvent
                (Query.find [])
                ( "submit", Encode.object [] )
                test
        )
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

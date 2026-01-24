module DashboardIntegrationTest exposing (suite)

{-| Integration tests for the Dashboard page using elm-program-test.
These tests verify user interactions, app management flows, and navigation.
-}

import Effect
import Expect
import Json.Decode as Decode
import Json.Encode as Encode
import Main exposing (Flags, Msg(..))
import ProgramTest exposing (ProgramTest)
import ProgramTestHelpers as Helpers
import SimulatedEffect.Ports
import Test exposing (..)


{-| Helper to create a program with an authenticated user already logged in.
-}
createAuthenticatedProgram : ProgramTest (Main.Model ()) Main.Msg (Effect.Effect Main.Msg)
createAuthenticatedProgram =
    let
        user =
            { email = "test@example.com"
            , fullName = "Test User"
            }

        accessToken = "test-access-token"
        refreshToken = "test-refresh-token"
    in
    -- Start with a token so we skip the login flow
    ProgramTest.createApplication
        { init = Main.initForTesting
        , update = Main.update
        , view = Main.view
        , onUrlChange = Main.UrlChanged
        , onUrlRequest = Main.LinkClicked
        }
        |> ProgramTest.withBaseUrl "http://localhost"
        |> ProgramTest.withSimulatedEffects (Helpers.createSimulateEffects ())
        |> ProgramTest.withSimulatedSubscriptions
            (\model ->
                SimulatedEffect.Ports.subscribe "refreshTokenReceived"
                    (Decode.maybe Decode.string)
                    Main.RefreshTokenReceived
            )
        |> ProgramTest.start { token = Just accessToken }
        -- App will verify the token
        |> Helpers.ensureHttpRequest "GET" "/api/auth/me"
        |> Helpers.simulateHttpOk "GET" "/api/auth/me"
            (Encode.object
                [ ( "user"
                  , Encode.object
                        [ ( "email", Encode.string user.email )
                        , ( "full_name", Encode.string user.fullName )
                        ]
                  )
                , ( "token", Encode.string accessToken )
                ]
            )
        |> ProgramTest.advanceTime 100
        -- Simulate refresh token port response
        |> ProgramTest.simulateIncomingPort "refreshTokenReceived" (Encode.string refreshToken)
        |> ProgramTest.advanceTime 100
        -- Should now be on dashboard
        |> Helpers.ensureBrowserUrlPath "/dashboard"


suite : Test
suite =
    describe "Dashboard Page Integration Tests"
        [ describe "Create App Flow"
            [ test "shows create app form when clicking New App button" <|
                \_ ->
                    createAuthenticatedProgram
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> Helpers.ensureViewHasText "My Apps"
                        |> Helpers.clickButton "+ New App"
                        |> Helpers.ensureViewHasText "Create New App"
                        |> Helpers.ensureViewHasText "Step 1: Name your app"
                        |> ProgramTest.done
            , test "validates app name is required" <|
                \_ ->
                    createAuthenticatedProgram
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> Helpers.clickButton "+ New App"
                        |> Helpers.submitForm
                        |> Helpers.ensureViewHasText "App name is required"
                        |> ProgramTest.done
            , test "creates app without repository" <|
                \_ ->
                    let
                        newApp =
                            { id = "app-123"
                            , name = "my-new-app"
                            , state = "stopped"
                            }
                    in
                    createAuthenticatedProgram
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> Helpers.ensureHttpRequest "GET" "/api/github/status"
                        |> Helpers.simulateHttpOk "GET" "/api/github/status" (Helpers.githubStatusJson False Nothing)
                        |> ProgramTest.advanceTime 100
                        |> Helpers.clickButton "+ New App"
                        |> Helpers.fillIn "appName" "my-new-app"
                        |> Helpers.submitForm
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureHttpRequest "GET" "/api/github/status"
                        |> Helpers.simulateHttpOk "GET" "/api/github/status" (Helpers.githubStatusJson False Nothing)
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureViewHasText "Step 2: Connect GitHub (Optional)"
                        |> Helpers.clickButton "Skip"
                        |> Helpers.ensureHttpRequestWithBody
                            "POST"
                            "/api/apps"
                            (Encode.object [ ( "name", Encode.string "my-new-app" ) ])
                        |> Helpers.simulateHttpOk "POST" "/api/apps"
                            (Encode.object
                                [ ( "id", Encode.string newApp.id )
                                , ( "name", Encode.string newApp.name )
                                , ( "state", Encode.string newApp.state )
                                ]
                            )
                        |> ProgramTest.advanceTime 300
                        |> Helpers.ensureViewHasText "My Apps"
                        -- App should be in the list (added to model.apps when created)
                        -- The app name appears in the h3 element inside the app card
                        |> Helpers.ensureViewHasText "my-new-app"
                        |> ProgramTest.done
            ]
        , describe "App List View"
            [ test "displays empty state when no apps exist" <|
                \_ ->
                    createAuthenticatedProgram
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> Helpers.ensureHttpRequest "GET" "/api/apps"
                        |> Helpers.simulateHttpOk "GET" "/api/apps" (Helpers.appsListJson [])
                        |> Helpers.ensureViewHasText "No apps yet"
                        |> Helpers.ensureViewHasText "Create your first app to get started"
                        |> ProgramTest.done
            , test "displays apps list when apps exist" <|
                \_ ->
                    let
                        apps =
                            [ { id = "app-1", name = "my-app", state = "stopped" }
                            , { id = "app-2", name = "another-app", state = "running" }
                            ]
                    in
                    createAuthenticatedProgram
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> Helpers.ensureHttpRequest "GET" "/api/apps"
                        |> Helpers.simulateHttpOk "GET" "/api/apps" (Helpers.appsListJson apps)
                        |> Helpers.ensureViewHasText "my-app"
                        |> Helpers.ensureViewHasText "another-app"
                        |> Helpers.ensureViewHasText "stopped"
                        |> Helpers.ensureViewHasText "running"
                        |> ProgramTest.done
            ]
        , describe "App Detail View"
            [ test "navigates to app detail when clicking app card" <|
                \_ ->
                    let
                        app =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "stopped"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }
                    in
                    createAuthenticatedProgram
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> Helpers.ensureHttpRequest "GET" "/api/apps"
                        |> Helpers.simulateHttpOk "GET" "/api/apps"
                            (Helpers.appsListJson
                                [ { id = "app-1", name = "my-app", state = "stopped" } ]
                            )
                        |> Helpers.clickLink "my-app"
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureHttpRequest "GET" "/api/apps/my-app"
                        |> Helpers.simulateHttpOk "GET" "/api/apps/my-app" (Helpers.appDetailJson app)
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureBrowserUrlPath "/apps/my-app"
                        |> Helpers.ensureViewHasText "my-app"
                        |> Helpers.ensureViewHasText "Information"
                        |> Helpers.ensureViewHasText "Actions"
                        |> ProgramTest.done
            , test "starts app when clicking Start button" <|
                \_ ->
                    let
                        app =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "stopped"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }
                    in
                    createAuthenticatedProgram
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> Helpers.ensureHttpRequest "GET" "/api/apps"
                        |> Helpers.simulateHttpOk "GET" "/api/apps"
                            (Helpers.appsListJson
                                [ { id = "app-1", name = "my-app", state = "stopped" } ]
                            )
                        |> Helpers.clickLink "my-app"
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureHttpRequest "GET" "/api/apps/my-app"
                        |> Helpers.simulateHttpOk "GET" "/api/apps/my-app" (Helpers.appDetailJson app)
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureBrowserUrlPath "/apps/my-app"
                        |> Helpers.clickButton "Start"
                        |> Helpers.ensureHttpRequest "POST" "/api/apps/my-app/start"
                        |> Helpers.simulateHttpOk "POST" "/api/apps/my-app/start" (Encode.string "App started")
                        |> Helpers.ensureHttpRequest "GET" "/api/apps/my-app"
                        |> ProgramTest.done
            , test "stops app when clicking Stop button" <|
                \_ ->
                    let
                        runningApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "running"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }
                    in
                    createAuthenticatedProgram
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> Helpers.ensureHttpRequest "GET" "/api/apps"
                        |> Helpers.simulateHttpOk "GET" "/api/apps"
                            (Helpers.appsListJson
                                [ { id = "app-1", name = "my-app", state = "running" } ]
                            )
                        |> Helpers.clickLink "my-app"
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureHttpRequest "GET" "/api/apps/my-app"
                        |> Helpers.simulateHttpOk "GET" "/api/apps/my-app" (Helpers.appDetailJson runningApp)
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureBrowserUrlPath "/apps/my-app"
                        |> Helpers.clickButton "Stop"
                        |> Helpers.ensureHttpRequest "POST" "/api/apps/my-app/stop"
                        |> Helpers.simulateHttpOk "POST" "/api/apps/my-app/stop" (Encode.string "App stopped")
                        |> Helpers.ensureHttpRequest "GET" "/api/apps/my-app"
                        |> ProgramTest.done
            , test "builds app when clicking Build button" <|
                \_ ->
                    let
                        app =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "stopped"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote =
                                Just
                                    { name = "origin"
                                    , url = "https://github.com/user/repo.git"
                                    , branch = "main"
                                    }
                            }
                    in
                    createAuthenticatedProgram
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> Helpers.ensureHttpRequest "GET" "/api/apps"
                        |> Helpers.simulateHttpOk "GET" "/api/apps"
                            (Helpers.appsListJson
                                [ { id = "app-1", name = "my-app", state = "stopped" } ]
                            )
                        |> Helpers.clickLink "my-app"
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureHttpRequest "GET" "/api/apps/my-app"
                        |> Helpers.simulateHttpOk "GET" "/api/apps/my-app" (Helpers.appDetailJson app)
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureBrowserUrlPath "/apps/my-app"
                        |> ProgramTest.advanceTime 50
                        |> Helpers.clickButton "Build"
                        |> ProgramTest.advanceTime 50
                        |> Helpers.ensureHttpRequest "POST" "/api/apps/my-app/build"
                        |> Helpers.simulateHttpOk "POST" "/api/apps/my-app/build"
                            (Encode.object
                                [ ( "message", Encode.string "Build started" )
                                , ( "build_id", Encode.string "build-123" )
                                ]
                            )
                        |> Helpers.ensureViewHasText "Building app"
                        |> ProgramTest.done
            ]
        , describe "Navigation"
            [ test "navigates back to apps list from app detail" <|
                \_ ->
                    let
                        app =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "stopped"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }
                    in
                    createAuthenticatedProgram
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> Helpers.ensureHttpRequest "GET" "/api/apps"
                        |> Helpers.simulateHttpOk "GET" "/api/apps"
                            (Helpers.appsListJson
                                [ { id = "app-1", name = "my-app", state = "stopped" } ]
                            )
                        |> Helpers.clickLink "my-app"
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureHttpRequest "GET" "/api/apps/my-app"
                        |> Helpers.simulateHttpOk "GET" "/api/apps/my-app" (Helpers.appDetailJson app)
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureBrowserUrlPath "/apps/my-app"
                        |> Helpers.clickButton "< Apps"
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> ProgramTest.done
            , test "logs out when clicking Logout button" <|
                \_ ->
                    createAuthenticatedProgram
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> Helpers.clickButton "Logout"
                        |> Helpers.ensureBrowserUrlPath "/login"
                        |> ProgramTest.done
            , test "updates SSE filters when viewing app detail" <|
                \_ ->
                    let
                        app =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "stopped"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        expectedFilter =
                            Encode.object
                                [ ( "app_names", Encode.list Encode.string [ "my-app" ] )
                                ]
                    in
                    createAuthenticatedProgram
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> Helpers.ensureHttpRequest "GET" "/api/apps"
                        |> Helpers.simulateHttpOk "GET" "/api/apps"
                            (Helpers.appsListJson
                                [ { id = "app-1", name = "my-app", state = "stopped" } ]
                            )
                        |> Helpers.clickLink "my-app"
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureHttpRequest "GET" "/api/apps/my-app"
                        |> Helpers.simulateHttpOk "GET" "/api/apps/my-app" (Helpers.appDetailJson app)
                        |> ProgramTest.advanceTime 100
                        |> ProgramTest.ensureOutgoingPortValues "updateSSEFilters"
                            (Decode.field "filters" Decode.value)
                            (\values ->
                                if List.any (\v -> Encode.encode 0 v == Encode.encode 0 expectedFilter) values then
                                    Expect.pass
                                else
                                    Expect.fail ("Expected filter not found in port values. Got: " ++ Debug.toString values)
                            )
                        |> ProgramTest.done
            , test "clears SSE filters when navigating back from app detail" <|
                \_ ->
                    let
                        app =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "stopped"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }
                    in
                    createAuthenticatedProgram
                        |> Helpers.ensureBrowserUrlPath "/dashboard"
                        |> Helpers.ensureHttpRequest "GET" "/api/apps"
                        |> Helpers.simulateHttpOk "GET" "/api/apps"
                            (Helpers.appsListJson
                                [ { id = "app-1", name = "my-app", state = "stopped" } ]
                            )
                        |> Helpers.clickLink "my-app"
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureHttpRequest "GET" "/api/apps/my-app"
                        |> Helpers.simulateHttpOk "GET" "/api/apps/my-app" (Helpers.appDetailJson app)
                        |> ProgramTest.advanceTime 100
                        |> Helpers.ensureBrowserUrlPath "/apps/my-app"
                        |> Helpers.clickButton "< Apps"
                        |> ProgramTest.advanceTime 100
                        |> ProgramTest.ensureOutgoingPortValues "updateSSEFilters"
                            (Decode.field "filters" Decode.value)
                            (\values ->
                                if List.any (\v -> Encode.encode 0 v == Encode.encode 0 Encode.null) values then
                                    Expect.pass
                                else
                                    Expect.fail ("Expected null filter not found in port values. Got: " ++ Debug.toString values)
                            )
                        |> ProgramTest.done
            ]
        ]

module DashboardIntegrationTest exposing (suite)

{-| Integration tests for the Dashboard page using elm-program-test.
These tests verify user interactions, app management flows, and navigation.
-}

import Effect
import Expect
import Json.Encode as Encode
import Main exposing (Flags, Msg(..))
import ProgramTest exposing (ProgramTest)
import ProgramTestHelpers as Helpers
import Test exposing (..)


{-| Helper to create a program with an authenticated user already logged in.
-}
createAuthenticatedProgram : ProgramTest Main.Model Main.Msg (Cmd Main.Msg)
createAuthenticatedProgram =
    Helpers.createTestProgram
        |> Helpers.expectHttpRequest "GET" "/api/auth/status"
        |> ProgramTest.simulateHttpOk
            (ProgramTest.HttpRequest
                { method = "GET"
                , url = "/api/auth/status"
                , body = ProgramTest.HttpBodyEmpty
                , headers = []
                }
            )
            (Helpers.serverStatusJson True "1.0.0")
        |> ProgramTest.advanceTime 100


suite : Test
suite =
    describe "Dashboard Page Integration Tests"
        [ describe "Create App Flow"
            [ test "shows create app form when clicking New App button" <|
                \_ ->
                    createAuthenticatedProgram
                        |> ProgramTest.visitUrl "/dashboard"
                        |> Helpers.expectViewHasText "My Apps"
                        |> Helpers.clickButton "+ New App"
                        |> Helpers.expectViewHasText "Create New App"
                        |> Helpers.expectViewHasText "Step 1: Name your app"
            , test "validates app name is required" <|
                \_ ->
                    createAuthenticatedProgram
                        |> ProgramTest.visitUrl "/dashboard"
                        |> Helpers.clickButton "+ New App"
                        |> Helpers.submitForm
                        |> Helpers.expectViewHasText "App name is required"
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
                        |> ProgramTest.visitUrl "/dashboard"
                        |> Helpers.clickButton "+ New App"
                        |> Helpers.fillIn "App Name" "my-new-app"
                        |> Helpers.submitForm
                        |> Helpers.expectHttpRequestWithBody
                            "POST"
                            "/api/apps"
                            (Encode.object [ ( "name", Encode.string "my-new-app" ) ])
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "POST"
                                , url = "/api/apps"
                                , body = ProgramTest.HttpBodyJson
                                    (Encode.object [ ( "name", Encode.string "my-new-app" ) ])
                                , headers = []
                                }
                            )
                            (Encode.object
                                [ ( "id", Encode.string newApp.id )
                                , ( "name", Encode.string newApp.name )
                                , ( "state", Encode.string newApp.state )
                                ]
                            )
                        |> Helpers.expectViewHasText "My Apps"
                        |> Helpers.expectViewHasText "my-new-app"
            ]
        , describe "App List View"
            [ test "displays empty state when no apps exist" <|
                \_ ->
                    createAuthenticatedProgram
                        |> ProgramTest.visitUrl "/dashboard"
                        |> Helpers.expectHttpRequest "GET" "/api/apps"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/apps"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.appsListJson [])
                        |> Helpers.expectViewHasText "No apps yet"
                        |> Helpers.expectViewHasText "Create your first app"
            , test "displays apps list when apps exist" <|
                \_ ->
                    let
                        apps =
                            [ { id = "app-1", name = "my-app", state = "stopped" }
                            , { id = "app-2", name = "another-app", state = "running" }
                            ]
                    in
                    createAuthenticatedProgram
                        |> ProgramTest.visitUrl "/dashboard"
                        |> Helpers.expectHttpRequest "GET" "/api/apps"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/apps"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.appsListJson apps)
                        |> Helpers.expectViewHasText "my-app"
                        |> Helpers.expectViewHasText "another-app"
                        |> Helpers.expectViewHasText "stopped"
                        |> Helpers.expectViewHasText "running"
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
                        |> ProgramTest.visitUrl "/dashboard"
                        |> Helpers.expectHttpRequest "GET" "/api/apps"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/apps"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.appsListJson
                                [ { id = "app-1", name = "my-app", state = "stopped" } ]
                            )
                        |> ProgramTest.clickButton "my-app"
                        |> Helpers.expectHttpRequest "GET" "/api/apps/my-app"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/apps/my-app"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.appDetailJson app)
                        |> Helpers.expectViewHasText "my-app"
                        |> Helpers.expectViewHasText "Information"
                        |> Helpers.expectViewHasText "Actions"
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
                        |> ProgramTest.visitUrl "/apps/my-app"
                        |> Helpers.expectHttpRequest "GET" "/api/apps/my-app"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/apps/my-app"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.appDetailJson app)
                        |> Helpers.clickButton "Start"
                        |> Helpers.expectHttpRequest "POST" "/api/apps/my-app/start"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "POST"
                                , url = "/api/apps/my-app/start"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Encode.string "App started")
                        |> Helpers.expectHttpRequest "GET" "/api/apps/my-app"
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
                        |> ProgramTest.visitUrl "/apps/my-app"
                        |> Helpers.expectHttpRequest "GET" "/api/apps/my-app"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/apps/my-app"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.appDetailJson runningApp)
                        |> Helpers.clickButton "Stop"
                        |> Helpers.expectHttpRequest "POST" "/api/apps/my-app/stop"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "POST"
                                , url = "/api/apps/my-app/stop"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Encode.string "App stopped")
                        |> Helpers.expectHttpRequest "GET" "/api/apps/my-app"
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
                        |> ProgramTest.visitUrl "/apps/my-app"
                        |> Helpers.expectHttpRequest "GET" "/api/apps/my-app"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/apps/my-app"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.appDetailJson app)
                        |> Helpers.clickButton "Build"
                        |> Helpers.expectHttpRequest "POST" "/api/apps/my-app/build"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "POST"
                                , url = "/api/apps/my-app/build"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Encode.object
                                [ ( "message", Encode.string "Build started" )
                                , ( "build_id", Encode.string "build-123" )
                                ]
                            )
                        |> Helpers.expectViewHasText "Building app"
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
                        |> ProgramTest.visitUrl "/apps/my-app"
                        |> Helpers.expectHttpRequest "GET" "/api/apps/my-app"
                        |> ProgramTest.simulateHttpOk
                            (ProgramTest.HttpRequest
                                { method = "GET"
                                , url = "/api/apps/my-app"
                                , body = ProgramTest.HttpBodyEmpty
                                , headers = []
                                }
                            )
                            (Helpers.appDetailJson app)
                        |> Helpers.clickButton "< Apps"
                        |> ProgramTest.expectPageChange "/dashboard"
            , test "logs out when clicking Logout button" <|
                \_ ->
                    createAuthenticatedProgram
                        |> ProgramTest.visitUrl "/dashboard"
                        |> Helpers.clickButton "Logout"
                        |> ProgramTest.expectPageChange "/login"
            ]
        ]

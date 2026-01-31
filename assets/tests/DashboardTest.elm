module DashboardTest exposing (suite)

{-| Tests for Page.Dashboard, specifically the GotGitHubStatus handler
and build status event handlers.

This test verifies the fix for bugs where:
1. GotGitHubStatus calculated a status but never used it
2. Build status events (failed/success) didn't update the UI state
3. Build logs error events didn't clear actionInProgress
-}

import Browser.Navigation as Nav
import Effect exposing (Effect)
import Expect exposing (Expectation)
import Page.Dashboard as Dashboard
import Page.Dashboard.EnvVars as EnvVars
import Shared
import Test exposing (..)


{-| Create a minimal Shared.Model for testing.
WARNING: This uses Debug.todo for navKey which will crash if evaluated.
The functions we're testing (handleBuildStatusEvent, handleBuildLogsEvent)
only use shared.token, so navKey should never be evaluated in these tests.
If tests fail with "TODO navKey", it means the code path is trying to use navKey.
-}
testSharedModel : Maybe String -> Shared.Model ()
testSharedModel maybeToken =
    -- For unit tests, we use () as navigationKey since these functions don't use navKey
    -- The functions only use shared.token, so navKey type doesn't matter
    { navKey = ()
    , currentRoute = Nothing
    , serverVersion = ""
    , user = Nothing
    , token = maybeToken
    , refreshToken = Nothing
    , githubStatus = Shared.GitHubUnknown
    , sseConnectionState = "connected"
    }


{-| Create a CreateAppState for testing.
-}
testCreateAppState : String -> Dashboard.CreateAppState
testCreateAppState appName =
    { appName = appName
    , step = Dashboard.CheckingGitHub
    , error = Nothing
    }


{-| Create a minimal AppDetailState for testing.
-}
testAppDetailState : Effect.AppDetail -> { app : Effect.AppDetail, logs : String, logsLoading : Bool, logsView : Dashboard.LogsView, builds : List Effect.BuildInfo, selectedBuildId : Maybe String, buildLogs : String, buildLogsLoading : Bool, actionInProgress : Maybe Dashboard.AppAction, error : Maybe String, streamingBuildId : Maybe String, buildLogsStreaming : Bool, envVarsModel : EnvVars.Model }
testAppDetailState app =
    { app = app
    , logs = ""
    , logsLoading = False
    , logsView = Dashboard.RuntimeLogs
    , builds = []
    , selectedBuildId = Nothing
    , buildLogs = ""
    , buildLogsLoading = False
    , actionInProgress = Nothing
    , error = Nothing
    , streamingBuildId = Nothing
    , buildLogsStreaming = False
    , envVarsModel = EnvVars.init
    }


{-| Helper to check if an effect contains UpdateGitHubStatus.
-}
effectContainsUpdateGitHubStatus : Effect msg -> Bool
effectContainsUpdateGitHubStatus effect =
    case effect of
        Effect.UpdateGitHubStatus _ ->
            True

        Effect.Batch effects ->
            List.any effectContainsUpdateGitHubStatus effects

        _ ->
            False


{-| Helper to check if an effect contains FetchRepos.
-}
effectContainsFetchRepos : Effect msg -> Bool
effectContainsFetchRepos effect =
    case effect of
        Effect.FetchRepos _ _ ->
            True

        Effect.Batch effects ->
            List.any effectContainsFetchRepos effects

        _ ->
            False


{-| Helper to check if an effect is Effect.none (or effectively empty).
-}
effectIsNone : Effect msg -> Bool
effectIsNone effect =
    case effect of
        Effect.None ->
            True

        Effect.Batch [] ->
            True

        _ ->
            False


{-| Check if a step is SelectRepo.
-}
isSelectRepoStep : Dashboard.CreateAppStep -> Bool
isSelectRepoStep step =
    case step of
        Dashboard.SelectRepo _ _ ->
            True

        _ ->
            False


{-| Check if a step is ConnectGitHub.
-}
isConnectGitHubStep : Dashboard.CreateAppStep -> Bool
isConnectGitHubStep step =
    case step of
        Dashboard.ConnectGitHub _ ->
            True

        _ ->
            False


{-| Check if a step is EnterName.
-}
isEnterNameStep : Dashboard.CreateAppStep -> Bool
isEnterNameStep step =
    case step of
        Dashboard.EnterName ->
            True

        _ ->
            False


suite : Test
suite =
    describe "Page.Dashboard"
        [ describe "handleGotGitHubStatus"
        [ describe "when GitHub is connected"
            [ test "transitions to SelectRepo step" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( newStep, _, _ ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    isSelectRepoStep newStep
                        |> Expect.equal True
                        |> Expect.onFail "should be SelectRepo step"
            , test "clears any previous error" <|
                \_ ->
                    let
                        createState =
                            { appName = "my-app"
                            , step = Dashboard.CheckingGitHub
                            , error = Just "some previous error"
                            }

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( _, newError, _ ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    Expect.equal Nothing newError
            , test "emits UpdateGitHubStatus effect" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( _, _, effect ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    effectContainsUpdateGitHubStatus effect
                        |> Expect.equal True
                        |> Expect.onFail "should emit UpdateGitHubStatus effect"
            , test "emits FetchRepos effect" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( _, _, effect ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    effectContainsFetchRepos effect
                        |> Expect.equal True
                        |> Expect.onFail "should emit FetchRepos effect"
            , test "does NOT return Effect.none (regression test for bug fix)" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( _, _, effect ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    effectIsNone effect
                        |> Expect.equal False
                        |> Expect.onFail "should NOT return Effect.none - this was the original bug"
            ]
        , describe "when GitHub is not connected"
            [ test "transitions to ConnectGitHub step" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = False
                            , username = Nothing
                            }

                        ( newStep, _, _ ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    isConnectGitHubStep newStep
                        |> Expect.equal True
                        |> Expect.onFail "should be ConnectGitHub step"
            , test "clears any previous error" <|
                \_ ->
                    let
                        createState =
                            { appName = "my-app"
                            , step = Dashboard.CheckingGitHub
                            , error = Just "some previous error"
                            }

                        response =
                            { connected = False
                            , username = Nothing
                            }

                        ( _, newError, _ ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    Expect.equal Nothing newError
            , test "emits UpdateGitHubStatus effect" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = False
                            , username = Nothing
                            }

                        ( _, _, effect ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    effectContainsUpdateGitHubStatus effect
                        |> Expect.equal True
                        |> Expect.onFail "should emit UpdateGitHubStatus effect"
            , test "does NOT return Effect.none (regression test for bug fix)" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = False
                            , username = Nothing
                            }

                        ( _, _, effect ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState (Just "test-token")
                    in
                    effectIsNone effect
                        |> Expect.equal False
                        |> Expect.onFail "should NOT return Effect.none - this was the original bug"
            ]
        , describe "when GitHub status check fails"
            [ test "transitions to EnterName step" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        ( newStep, _, _ ) =
                            Dashboard.handleGotGitHubStatus (Err "Network error") createState (Just "test-token")
                    in
                    isEnterNameStep newStep
                        |> Expect.equal True
                        |> Expect.onFail "should be EnterName step"
            , test "sets error message" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        ( _, newError, _ ) =
                            Dashboard.handleGotGitHubStatus (Err "Network error") createState (Just "test-token")
                    in
                    Expect.equal (Just "Failed to check GitHub connection") newError
            ]
        , describe "when no token is available"
            [ test "keeps the current step unchanged" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( newStep, _, _ ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState Nothing
                    in
                    -- Should stay on CheckingGitHub step since no token
                    Expect.equal Dashboard.CheckingGitHub newStep
            , test "returns Effect.none" <|
                \_ ->
                    let
                        createState =
                            testCreateAppState "my-app"

                        response =
                            { connected = True
                            , username = Just "testuser"
                            }

                        ( _, _, effect ) =
                            Dashboard.handleGotGitHubStatus (Ok response) createState Nothing
                    in
                    effectIsNone effect
                        |> Expect.equal True
                        |> Expect.onFail "should return Effect.none when no token"
            ]
        ]
        {- NOTE: The handleBuildStatusEvent and handleBuildLogsEvent tests below
           require a Shared.Model with a Browser.Navigation.Key, which is opaque
           and can't be created in unit tests. These tests use Debug.todo for navKey.
           The functions being tested only use shared.token, so navKey should never
           be evaluated. If these tests fail with "TODO navKey", it indicates the
           code path is trying to use navKey unexpectedly.
           
           TODO: Convert these to integration tests using elm-program-test which
           can provide a real navKey, or refactor to make navKey optional for testing.
        -}
        , describe "handleBuildStatusEvent"
        [ describe "when build status is 'failed'"
            [ test "updates app state to 'failed' in apps list" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            }

                        model =
                            { view = Dashboard.AppsListView
                            , apps = [ testApp ]
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( updatedModel, _ ) =
                            Dashboard.handleBuildStatusEvent model shared "my-app" "build-123" "failed"
                    in
                    case updatedModel.apps of
                        [ app ] ->
                            Expect.equal "failed" app.state

                        _ ->
                            Expect.fail "Expected exactly one app in the list"
            , test "clears actionInProgress when viewing app detail" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            , port_ = Nothing
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            testAppDetailState testApp
                                |> (\s -> { s | actionInProgress = Just Dashboard.Building })

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( updatedModel, _ ) =
                            Dashboard.handleBuildStatusEvent model shared "my-app" "build-123" "failed"
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.equal Nothing updatedDetailState.actionInProgress

                        _ ->
                            Expect.fail "Expected AppDetailView"
            , test "emits FetchAppDetail and FetchBuilds effects when viewing app detail" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            , port_ = Nothing
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            testAppDetailState testApp
                                |> (\s -> { s | actionInProgress = Just Dashboard.Building })

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( _, effect ) =
                            Dashboard.handleBuildStatusEvent model shared "my-app" "build-123" "failed"
                    in
                    Expect.all
                        [ \_ -> effectContainsFetchAppDetail effect |> Expect.equal True
                        , \_ -> effectContainsFetchBuilds effect |> Expect.equal True
                        ]
                        ()
            , test "emits FetchApps effect when on apps list view" <|
                \_ ->
                    let
                        model =
                            { view = Dashboard.AppsListView
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( _, effect ) =
                            Dashboard.handleBuildStatusEvent model shared "my-app" "build-123" "failed"
                    in
                    effectContainsFetchApps effect
                        |> Expect.equal True
                        |> Expect.onFail "should emit FetchApps effect when on apps list view"
            ]
        , describe "when build status is 'success'"
            [ test "updates app state from 'building' to 'stopped' in apps list" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            }

                        model =
                            { view = Dashboard.AppsListView
                            , apps = [ testApp ]
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( updatedModel, _ ) =
                            Dashboard.handleBuildStatusEvent model shared "my-app" "build-123" "success"
                    in
                    case updatedModel.apps of
                        [ app ] ->
                            Expect.equal "stopped" app.state

                        _ ->
                            Expect.fail "Expected exactly one app in the list"
            , test "clears actionInProgress when viewing app detail" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            , port_ = Nothing
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            testAppDetailState testApp
                                |> (\s -> { s | actionInProgress = Just Dashboard.Building })

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( updatedModel, _ ) =
                            Dashboard.handleBuildStatusEvent model shared "my-app" "build-123" "success"
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.equal Nothing updatedDetailState.actionInProgress

                        _ ->
                            Expect.fail "Expected AppDetailView"
            ]
        ]
    , describe "Page.Dashboard.handleBuildLogsEvent"
        [ describe "when event type is 'error'"
            [ test "clears actionInProgress" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            , port_ = Nothing
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            testAppDetailState testApp
                                |> (\s -> 
                                    { s 
                                    | logsView = Dashboard.BuildLogs
                                    , actionInProgress = Just Dashboard.Building
                                    , streamingBuildId = Just "build-123"
                                    , buildLogsStreaming = True
                                    }
                                )

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( updatedModel, _ ) =
                            Dashboard.handleBuildLogsEvent model shared "my-app" "build-123" "error" "Build failed"
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.all
                                [ \_ -> Expect.equal Nothing updatedDetailState.actionInProgress
                                , \_ -> Expect.equal False updatedDetailState.buildLogsStreaming
                                , \_ -> Expect.equal Nothing updatedDetailState.streamingBuildId
                                , \_ -> Expect.equal (Just "Build error: Build failed") updatedDetailState.error
                                ]
                                ()

                        _ ->
                            Expect.fail "Expected AppDetailView"
            , test "emits FetchAppDetail and FetchBuilds effects" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "building"
                            , port_ = Nothing
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            testAppDetailState testApp
                                |> (\s -> 
                                    { s 
                                    | logsView = Dashboard.BuildLogs
                                    , actionInProgress = Just Dashboard.Building
                                    , streamingBuildId = Just "build-123"
                                    , buildLogsStreaming = True
                                    }
                                )

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( _, effect ) =
                            Dashboard.handleBuildLogsEvent model shared "my-app" "build-123" "error" "Build failed"
                    in
                    Expect.all
                        [ \_ -> effectContainsFetchAppDetail effect |> Expect.equal True
                        , \_ -> effectContainsFetchBuilds effect |> Expect.equal True
                        ]
                        ()
            ]
        ]
    , describe "Page.Dashboard.handleContainerLogsEvent"
        [ test "appends log line when viewing matching app" <|
            \_ ->
                let
                    testApp =
                        { id = "app-1"
                        , name = "my-app"
                        , state = "running"
                        , port_ = Just 8080
                        , createdAt = "2024-01-01T00:00:00Z"
                        , updatedAt = "2024-01-01T00:00:00Z"
                        , remote = Nothing
                        }

                    detailState =
                        testAppDetailState testApp
                            |> (\s -> { s | logs = "existing log line" })

                    model =
                        { view = Dashboard.AppDetailView detailState
                        , apps = []
                        , appsLoading = False
                        , activeSidebarItem = Dashboard.MyApps
                        }

                    shared =
                        testSharedModel (Just "test-token")

                    ( updatedModel, _ ) =
                        Dashboard.handleContainerLogsEvent model shared "my-app" "new log line"
                in
                case updatedModel.view of
                    Dashboard.AppDetailView updatedDetailState ->
                        Expect.equal "existing log line\nnew log line" updatedDetailState.logs

                    _ ->
                        Expect.fail "Expected AppDetailView"
        , test "sets log line when logs are empty" <|
            \_ ->
                let
                    testApp =
                        { id = "app-1"
                        , name = "my-app"
                        , state = "running"
                        , port_ = Just 8080
                        , createdAt = "2024-01-01T00:00:00Z"
                        , updatedAt = "2024-01-01T00:00:00Z"
                        , remote = Nothing
                        }

                    detailState =
                        testAppDetailState testApp

                    model =
                        { view = Dashboard.AppDetailView detailState
                        , apps = []
                        , appsLoading = False
                        , activeSidebarItem = Dashboard.MyApps
                        }

                    shared =
                        testSharedModel (Just "test-token")

                    ( updatedModel, _ ) =
                        Dashboard.handleContainerLogsEvent model shared "my-app" "first log line"
                in
                case updatedModel.view of
                    Dashboard.AppDetailView updatedDetailState ->
                        Expect.equal "first log line" updatedDetailState.logs

                    _ ->
                        Expect.fail "Expected AppDetailView"
        , test "ignores logs for different app" <|
            \_ ->
                let
                    testApp =
                        { id = "app-1"
                        , name = "my-app"
                        , state = "running"
                        , port_ = Just 8080
                        , createdAt = "2024-01-01T00:00:00Z"
                        , updatedAt = "2024-01-01T00:00:00Z"
                        , remote = Nothing
                        }

                    detailState =
                        testAppDetailState testApp
                            |> (\s -> { s | logs = "existing logs" })

                    model =
                        { view = Dashboard.AppDetailView detailState
                        , apps = []
                        , appsLoading = False
                        , activeSidebarItem = Dashboard.MyApps
                        }

                    shared =
                        testSharedModel (Just "test-token")

                    ( updatedModel, _ ) =
                        Dashboard.handleContainerLogsEvent model shared "other-app" "should be ignored"
                in
                case updatedModel.view of
                    Dashboard.AppDetailView updatedDetailState ->
                        Expect.equal "existing logs" updatedDetailState.logs

                    _ ->
                        Expect.fail "Expected AppDetailView"
        , test "returns no effect" <|
            \_ ->
                let
                    testApp =
                        { id = "app-1"
                        , name = "my-app"
                        , state = "running"
                        , port_ = Just 8080
                        , createdAt = "2024-01-01T00:00:00Z"
                        , updatedAt = "2024-01-01T00:00:00Z"
                        , remote = Nothing
                        }

                    detailState =
                        testAppDetailState testApp

                    model =
                        { view = Dashboard.AppDetailView detailState
                        , apps = []
                        , appsLoading = False
                        , activeSidebarItem = Dashboard.MyApps
                        }

                    shared =
                        testSharedModel (Just "test-token")

                    ( _, effect ) =
                        Dashboard.handleContainerLogsEvent model shared "my-app" "log line"
                in
                Expect.equal True (effectIsNone effect)
        ]
    , describe "Page.Dashboard.GotAppDetail"
        [ test "triggers StartLogStreaming when app is running" <|
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

                    model =
                        { view = Dashboard.AppsListView
                        , apps = []
                        , appsLoading = False
                        , activeSidebarItem = Dashboard.MyApps
                        }

                    shared =
                        testSharedModel (Just "test-token")

                    ( _, effect ) =
                        Dashboard.update shared (Dashboard.GotAppDetail (Ok runningApp)) model
                in
                Expect.equal True (effectContainsStartLogStreaming effect)
        , test "does not trigger StartLogStreaming when app is stopped" <|
            \_ ->
                let
                    stoppedApp =
                        { id = "app-1"
                        , name = "my-app"
                        , state = "stopped"
                        , port_ = Just 8080
                        , createdAt = "2024-01-01T00:00:00Z"
                        , updatedAt = "2024-01-01T00:00:00Z"
                        , remote = Nothing
                        }

                    model =
                        { view = Dashboard.AppsListView
                        , apps = []
                        , appsLoading = False
                        , activeSidebarItem = Dashboard.MyApps
                        }

                    shared =
                        testSharedModel (Just "test-token")

                    ( _, effect ) =
                        Dashboard.update shared (Dashboard.GotAppDetail (Ok stoppedApp)) model
                in
                Expect.equal False (effectContainsStartLogStreaming effect)
        , test "does not trigger StartLogStreaming when no token" <|
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

                    model =
                        { view = Dashboard.AppsListView
                        , apps = []
                        , appsLoading = False
                        , activeSidebarItem = Dashboard.MyApps
                        }

                    shared =
                        testSharedModel Nothing

                    ( _, effect ) =
                        Dashboard.update shared (Dashboard.GotAppDetail (Ok runningApp)) model
                in
                Expect.equal False (effectContainsStartLogStreaming effect)
        ]
    , describe "Page.Dashboard Environment Variables"
        [ describe "EnvVarKeyChanged"
            [ test "updates the key in the form" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "running"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Nothing
                            , error = Nothing
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            , envVarsModel = EnvVars.init
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel Nothing

                        ( updatedModel, _ ) =
                            Dashboard.update shared (Dashboard.EnvVarsMsg (EnvVars.EnvVarKeyChanged "NEW_KEY")) model
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.equal "NEW_KEY" updatedDetailState.envVarsModel.envVarForm.key

                        _ ->
                            Expect.fail "Expected AppDetailView"
            ]
        , describe "EnvVarValueChanged"
            [ test "updates the value in the form" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "running"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Nothing
                            , error = Nothing
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            , envVarsModel =
                                { envVars = []
                                , envVarsLoading = False
                                , envVarForm = { key = "KEY", value = "", editingKey = Nothing }
                                }
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel Nothing

                        ( updatedModel, _ ) =
                            Dashboard.update shared (Dashboard.EnvVarsMsg (EnvVars.EnvVarValueChanged "new_value")) model
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.equal "new_value" updatedDetailState.envVarsModel.envVarForm.value

                        _ ->
                            Expect.fail "Expected AppDetailView"
            ]
        , describe "GotEnvVars"
            [ test "updates env vars list" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "running"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Nothing
                            , error = Nothing
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            , envVarsModel =
                                { envVars = []
                                , envVarsLoading = True
                                , envVarForm = { key = "", value = "", editingKey = Nothing }
                                }
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel Nothing

                        envVars =
                            [ { key = "KEY1", value = "value1" }
                            , { key = "KEY2", value = "value2" }
                            ]

                        ( updatedModel, _ ) =
                            Dashboard.update shared (Dashboard.EnvVarsMsg (EnvVars.GotEnvVars (Ok envVars))) model
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.all
                                [ \_ -> Expect.equal 2 (List.length updatedDetailState.envVarsModel.envVars)
                                , \_ -> Expect.equal False updatedDetailState.envVarsModel.envVarsLoading
                                ]
                                ()

                        _ ->
                            Expect.fail "Expected AppDetailView"
            , test "handles error when fetching env vars fails" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "running"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Nothing
                            , error = Nothing
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            , envVarsModel =
                                { envVars = []
                                , envVarsLoading = True
                                , envVarForm = { key = "", value = "", editingKey = Nothing }
                                }
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel Nothing

                        ( updatedModel, _ ) =
                            Dashboard.update shared (Dashboard.EnvVarsMsg (EnvVars.GotEnvVars (Err "Network error"))) model
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.equal False updatedDetailState.envVarsModel.envVarsLoading

                        _ ->
                            Expect.fail "Expected AppDetailView"
            ]
        , describe "SubmitEnvVar"
            [ test "validates that key is required" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "running"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Nothing
                            , error = Nothing
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            , envVarsModel =
                                { envVars = []
                                , envVarsLoading = False
                                , envVarForm = { key = "", value = "some value", editingKey = Nothing }
                                }
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( updatedModel, _ ) =
                            Dashboard.update shared (Dashboard.EnvVarsMsg EnvVars.SubmitEnvVar) model
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.equal (Just "Environment variable key is required") updatedDetailState.error

                        _ ->
                            Expect.fail "Expected AppDetailView"
            , test "emits SetEnvVar effect when key is provided" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "running"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Nothing
                            , error = Nothing
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            , envVarsModel =
                                { envVars = []
                                , envVarsLoading = False
                                , envVarForm = { key = "NEW_KEY", value = "new_value", editingKey = Nothing }
                                }
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( _, effect ) =
                            Dashboard.update shared (Dashboard.EnvVarsMsg EnvVars.SubmitEnvVar) model
                    in
                    effectContainsSetEnvVar effect
                        |> Expect.equal True
                        |> Expect.onFail "should emit SetEnvVar effect"
            ]
        , describe "CancelEnvVarEdit"
            [ test "clears the form" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "running"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Nothing
                            , error = Just "some error"
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            , envVarsModel =
                                { envVars = []
                                , envVarsLoading = False
                                , envVarForm = { key = "KEY", value = "value", editingKey = Just "KEY" }
                                }
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel Nothing

                        ( updatedModel, _ ) =
                            Dashboard.update shared (Dashboard.EnvVarsMsg EnvVars.CancelEnvVarEdit) model
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.all
                                [ \_ -> Expect.equal "" updatedDetailState.envVarsModel.envVarForm.key
                                , \_ -> Expect.equal "" updatedDetailState.envVarsModel.envVarForm.value
                                , \_ -> Expect.equal Nothing updatedDetailState.envVarsModel.envVarForm.editingKey
                                , \_ -> Expect.equal Nothing updatedDetailState.error
                                ]
                                ()

                        _ ->
                            Expect.fail "Expected AppDetailView"
            ]
        , describe "EditEnvVar"
            [ test "populates form with existing env var" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "running"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Nothing
                            , error = Nothing
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            , envVarsModel =
                                { envVars = [ { key = "EXISTING_KEY", value = "existing_value" } ]
                                , envVarsLoading = False
                                , envVarForm = { key = "", value = "", editingKey = Nothing }
                                }
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel Nothing

                        ( updatedModel, _ ) =
                            Dashboard.update shared (Dashboard.EnvVarsMsg (EnvVars.EditEnvVar "EXISTING_KEY")) model
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.all
                                [ \_ -> Expect.equal "EXISTING_KEY" updatedDetailState.envVarsModel.envVarForm.key
                                , \_ -> Expect.equal "existing_value" updatedDetailState.envVarsModel.envVarForm.value
                                , \_ -> Expect.equal (Just "EXISTING_KEY") updatedDetailState.envVarsModel.envVarForm.editingKey
                                ]
                                ()

                        _ ->
                            Expect.fail "Expected AppDetailView"
            ]
        , describe "DeleteEnvVar"
            [ test "emits SetEnvVar effect with delete flag" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "running"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Nothing
                            , error = Nothing
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            , envVarsModel =
                                { envVars = []
                                , envVarsLoading = False
                                , envVarForm = { key = "KEY", value = "value", editingKey = Just "KEY" }
                                }
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( _, effect ) =
                            Dashboard.update shared (Dashboard.EnvVarsMsg (EnvVars.DeleteEnvVar "KEY_TO_DELETE")) model
                    in
                    effectContainsSetEnvVar effect
                        |> Expect.equal True
                        |> Expect.onFail "should emit SetEnvVar effect for deletion"
            ]
        , describe "GotEnvVarSet"
            [ test "refetches env vars on success" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "running"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Nothing
                            , error = Nothing
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            , envVarsModel =
                                { envVars = []
                                , envVarsLoading = False
                                , envVarForm = { key = "", value = "", editingKey = Nothing }
                                }
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( _, effect ) =
                            Dashboard.update shared (Dashboard.EnvVarsMsg (EnvVars.GotEnvVarSet (Ok "Success"))) model
                    in
                    effectContainsFetchEnvVars effect
                        |> Expect.equal True
                        |> Expect.onFail "should emit FetchEnvVars effect after successful set"
            , test "sets error on failure" <|
                \_ ->
                    let
                        testApp =
                            { id = "app-1"
                            , name = "my-app"
                            , state = "running"
                            , port_ = Just 8080
                            , createdAt = "2024-01-01T00:00:00Z"
                            , updatedAt = "2024-01-01T00:00:00Z"
                            , remote = Nothing
                            }

                        detailState =
                            { app = testApp
                            , logs = ""
                            , logsLoading = False
                            , logsView = Dashboard.RuntimeLogs
                            , builds = []
                            , selectedBuildId = Nothing
                            , buildLogs = ""
                            , buildLogsLoading = False
                            , actionInProgress = Nothing
                            , error = Nothing
                            , streamingBuildId = Nothing
                            , buildLogsStreaming = False
                            , envVarsModel =
                                { envVars = []
                                , envVarsLoading = False
                                , envVarForm = { key = "", value = "", editingKey = Nothing }
                                }
                            }

                        model =
                            { view = Dashboard.AppDetailView detailState
                            , apps = []
                            , appsLoading = False
                            , activeSidebarItem = Dashboard.MyApps
                            }

                        shared =
                            testSharedModel (Just "test-token")

                        ( updatedModel, _ ) =
                            Dashboard.update shared (Dashboard.EnvVarsMsg (EnvVars.GotEnvVarSet (Err "Failed to set env var"))) model
                    in
                    case updatedModel.view of
                        Dashboard.AppDetailView updatedDetailState ->
                            Expect.equal (Just "Failed to set env var") updatedDetailState.error

                        _ ->
                            Expect.fail "Expected AppDetailView"
            ]
        ]
    ]


{-| Helper to check if an effect contains FetchAppDetail.
-}
effectContainsFetchAppDetail : Effect msg -> Bool
effectContainsFetchAppDetail effect =
    case effect of
        Effect.FetchAppDetail _ _ _ ->
            True

        Effect.Batch effects ->
            List.any effectContainsFetchAppDetail effects

        _ ->
            False


{-| Helper to check if an effect contains FetchBuilds.
-}
effectContainsFetchBuilds : Effect msg -> Bool
effectContainsFetchBuilds effect =
    case effect of
        Effect.FetchBuilds _ _ _ ->
            True

        Effect.Batch effects ->
            List.any effectContainsFetchBuilds effects

        _ ->
            False


{-| Helper to check if an effect contains FetchApps.
-}
effectContainsFetchApps : Effect msg -> Bool
effectContainsFetchApps effect =
    case effect of
        Effect.FetchApps _ _ ->
            True

        Effect.Batch effects ->
            List.any effectContainsFetchApps effects

        _ ->
            False


{-| Helper to check if an effect contains StartLogStreaming.
-}
effectContainsStartLogStreaming : Effect msg -> Bool
effectContainsStartLogStreaming effect =
    case effect of
        Effect.StartLogStreaming _ _ _ ->
            True

        Effect.Batch effects ->
            List.any effectContainsStartLogStreaming effects

        _ ->
            False


{-| Helper to check if an effect contains SetEnvVar.
-}
effectContainsSetEnvVar : Effect msg -> Bool
effectContainsSetEnvVar effect =
    case effect of
        Effect.SetEnvVar _ _ _ _ _ _ ->
            True

        Effect.Batch effects ->
            List.any effectContainsSetEnvVar effects

        _ ->
            False


{-| Helper to check if an effect contains FetchEnvVars.
-}
effectContainsFetchEnvVars : Effect msg -> Bool
effectContainsFetchEnvVars effect =
    case effect of
        Effect.FetchEnvVars _ _ _ ->
            True

        Effect.Batch effects ->
            List.any effectContainsFetchEnvVars effects

        _ ->
            False

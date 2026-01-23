module DashboardTest exposing (suite)

{-| Tests for Page.Dashboard, specifically the GotGitHubStatus handler.

This test verifies the fix for a bug where GotGitHubStatus calculated a status
but never used it - it just returned (model, Effect.none) without:
1. Updating the shared GitHub status
2. Transitioning from the CheckingGitHub step to the next step
-}

import Effect exposing (Effect)
import Expect exposing (Expectation)
import Page.Dashboard as Dashboard
import Test exposing (..)


{-| Create a CreateAppState for testing.
-}
testCreateAppState : String -> Dashboard.CreateAppState
testCreateAppState appName =
    { appName = appName
    , step = Dashboard.CheckingGitHub
    , error = Nothing
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
    describe "Page.Dashboard.handleGotGitHubStatus"
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

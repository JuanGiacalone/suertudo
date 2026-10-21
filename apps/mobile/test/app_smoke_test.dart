import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:suertudo_mobile/main.dart';

/// The mobile app has no behavior of its own yet: the screens come with the
/// first epic of Fase 1 (see `TSK-F1-07`). What this test holds is that the
/// app COMES UP: the dependency graph resolves, the entry point runs and the
/// root widget renders.
///
/// It is the mobile counterpart of `GET /v1/health`.
void main() {
  testWidgets('the app comes up and renders its root widget', (tester) async {
    await tester.pumpWidget(const MyApp());

    // A check that only looks for the absence of an error cannot tell
    // "everything is fine" from "nothing ran": it reports PASS in both
    // cases. The widget has to be THERE.
    expect(find.byType(MaterialApp), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
